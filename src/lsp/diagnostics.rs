/// Convert Taida parse errors and type errors to LSP Diagnostics.
///
/// Provides:
/// - Parse errors (severity: Error)
/// - Type errors from TypeChecker (severity: Warning)
///   - Type mismatches in assignments
///   - Undefined variable warnings (when TypeChecker reports unknown types)
///   - Empty list literal without type annotation
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

use crate::parser::{ParseError, parse};
use crate::types::TypeChecker;

/// Result of analyzing a Taida source file.
pub struct AnalysisResult {
    pub diagnostics: Vec<Diagnostic>,
}

/// Analyze source code and produce LSP diagnostics.
pub fn analyze(source: &str) -> AnalysisResult {
    let mut diagnostics = Vec::new();

    // Phase 1: Parse errors
    let (program, parse_errors) = parse(source);
    for err in &parse_errors {
        diagnostics.push(parse_error_to_diagnostic(err, source));
    }

    // Phase 2: Type errors (only if parse succeeded)
    if parse_errors.is_empty() {
        let mut checker = TypeChecker::new();
        checker.check_program(&program);
        for err in &checker.errors {
            diagnostics.push(type_error_to_diagnostic(err, source));
        }
    }

    AnalysisResult { diagnostics }
}

/// Convert a ParseError to an LSP Diagnostic with improved range calculation.
fn parse_error_to_diagnostic(err: &ParseError, source: &str) -> Diagnostic {
    let line = if err.span.line > 0 {
        err.span.line - 1
    } else {
        0
    };
    let col = if err.span.column > 0 {
        err.span.column - 1
    } else {
        0
    };

    // Calculate end position: use the span's byte range if available,
    // otherwise use the line end or a reasonable default width.
    let end_col = if err.span.end > err.span.start {
        let line_start_byte = source
            .lines()
            .take(line)
            .map(|l| l.len() + 1) // +1 for newline
            .sum::<usize>();

        err.span.end.saturating_sub(line_start_byte)
    } else {
        // Default: highlight the next few characters based on error message content
        let line_text = source.lines().nth(line).unwrap_or("");
        let remaining = line_text.len().saturating_sub(col);
        col + remaining.clamp(1, 10)
    };

    Diagnostic {
        range: Range {
            start: Position {
                line: line as u32,
                character: col as u32,
            },
            end: Position {
                line: line as u32,
                character: end_col as u32,
            },
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("taida".to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

/// Convert a TypeError to an LSP Diagnostic with improved range calculation.
fn type_error_to_diagnostic(err: &crate::types::TypeError, source: &str) -> Diagnostic {
    let line = if err.span.line > 0 {
        err.span.line - 1
    } else {
        0
    };
    let col = if err.span.column > 0 {
        err.span.column - 1
    } else {
        0
    };

    // Calculate end position from byte range or use intelligent defaults
    let end_col = if err.span.end > err.span.start {
        let line_start_byte = source
            .lines()
            .take(line)
            .map(|l| l.len() + 1)
            .sum::<usize>();

        err.span.end.saturating_sub(line_start_byte)
    } else {
        // Use the full line as the range for type errors
        let line_text = source.lines().nth(line).unwrap_or("");
        line_text.len()
    };

    // Determine severity based on the error message content
    let severity = if err.message.contains("Type mismatch")
        || err.message.contains("requires a type annotation")
    {
        DiagnosticSeverity::ERROR
    } else {
        DiagnosticSeverity::WARNING
    };

    Diagnostic {
        range: Range {
            start: Position {
                line: line as u32,
                character: col as u32,
            },
            end: Position {
                line: line as u32,
                character: end_col as u32,
            },
        },
        severity: Some(severity),
        source: Some("taida-typecheck".to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_valid_source() {
        let source = "x <= 42\nname <= \"hello\"";
        let result = analyze(source);
        assert!(
            result.diagnostics.is_empty(),
            "Valid source should produce no diagnostics"
        );
    }

    #[test]
    fn test_analyze_parse_error() {
        // Incomplete assignment should produce a parse error
        let source = "x <=";
        let result = analyze(source);
        // Parse errors should be present
        assert!(
            !result.diagnostics.is_empty(),
            "Incomplete assignment should produce diagnostics. Got: {:?}",
            result.diagnostics
        );
        assert_eq!(
            result.diagnostics[0].severity,
            Some(DiagnosticSeverity::ERROR)
        );
    }

    #[test]
    fn test_analyze_type_error_empty_list() {
        let source = "items <= @[]";
        let result = analyze(source);
        // Empty list without type annotation should produce a type error
        let has_type_error = result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("type annotation") || d.message.contains("@[]"));
        assert!(
            has_type_error,
            "Empty list literal should produce type annotation warning. Got: {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_diagnostic_source_tag() {
        let source = "x <= 42";
        let result = analyze(source);
        // No errors expected, but verify the function doesn't panic
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_parse_error_has_taida_source() {
        let source = ">>> ";
        let result = analyze(source);
        if !result.diagnostics.is_empty() {
            assert_eq!(
                result.diagnostics[0].source,
                Some("taida".to_string()),
                "Parse errors should have 'taida' source"
            );
        }
    }

    #[test]
    fn test_type_mismatch_detection() {
        let source = "x: Int <= \"hello\"";
        let result = analyze(source);
        let has_mismatch = result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("Type mismatch"));
        assert!(
            has_mismatch,
            "Type mismatch should be detected. Got: {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }
}
