//! URL template variable expansion for prebuild addon downloads.
//!
//! Supports `{version}`, `{target}`, `{ext}`, `{name}` variables.
//! Unknown variables and unbalanced braces are rejected.

use std::fmt;

const VALID_VARIABLES: &[&str] = &["version", "target", "ext", "name"];

/// Errors from URL template expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UrlTemplateError {
    /// Unknown variable name in braces.
    UnknownVariable { variable: String },
    /// Unbalanced brace (missing closing `}`).
    UnbalancedBrace { detail: String },
}

impl fmt::Display for UrlTemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVariable { variable } => {
                write!(
                    f,
                    "url template error: unknown variable '{{{}}}' (supported: version, target, ext, name)",
                    variable
                )
            }
            Self::UnbalancedBrace { detail } => {
                write!(f, "url template error: unbalanced brace: {}", detail)
            }
        }
    }
}

impl std::error::Error for UrlTemplateError {}

/// Expands URL template variables.
///
/// `version` -> `{version}`, `target` -> `{target}`, `ext` -> `{ext}`, `name` -> `{name}`.
pub fn expand_url_template(
    template: &str,
    version: &str,
    target: &str,
    ext: &str,
    name: &str,
) -> Result<String, UrlTemplateError> {
    validate_template(template)?;

    let mut result = String::with_capacity(template.len() * 2);
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            let mut variable = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(ch) => variable.push(ch),
                    None => {
                        // Unbalanced: no closing brace
                        return Err(UrlTemplateError::UnbalancedBrace {
                            detail: format!("missing '}}' for '{{{variable}'"),
                        });
                    }
                }
            }

            // Empty variable name like "{}" is also treated as unknown
            let value = match variable.as_str() {
                "version" => version,
                "target" => target,
                "ext" => ext,
                "name" => name,
                other => {
                    return Err(UrlTemplateError::UnknownVariable {
                        variable: other.to_string(),
                    });
                }
            };
            result.push_str(value);
        } else if c == '}' {
            return Err(UrlTemplateError::UnbalancedBrace {
                detail: "unexpected '}' without opening '{'".to_string(),
            });
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Validates that a template only contains known variables and balanced braces.
pub fn validate_template(template: &str) -> Result<(), UrlTemplateError> {
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            // Check for escape sequence (v1 does not support {{ / }})
            if chars.peek() == Some(&'{') {
                return Err(UrlTemplateError::UnbalancedBrace {
                    detail: "escape sequences '{{' are not supported in v1".to_string(),
                });
            }

            let mut variable = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(ch) => variable.push(ch),
                    None => {
                        return Err(UrlTemplateError::UnbalancedBrace {
                            detail: format!("missing '}}' for '{{{variable}'"),
                        });
                    }
                }
            }

            if !VALID_VARIABLES.contains(&variable.as_str()) {
                return Err(UrlTemplateError::UnknownVariable {
                    variable: variable.clone(),
                });
            }
        } else if c == '}' {
            return Err(UrlTemplateError::UnbalancedBrace {
                detail: "unexpected '}' without opening '{'".to_string(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_all_variables() {
        let result = expand_url_template(
            "https://example.com/v{version}/lib{name}-{target}.{ext}",
            "1.0.0",
            "x86_64-unknown-linux-gnu",
            "so",
            "terminal",
        )
        .unwrap();
        assert_eq!(
            result,
            "https://example.com/v1.0.0/libterminal-x86_64-unknown-linux-gnu.so"
        );
    }

    #[test]
    fn expand_single_variable() {
        let result = expand_url_template("{version}", "a.1", "target", "ext", "name").unwrap();
        assert_eq!(result, "a.1");
    }

    #[test]
    fn expand_no_variables() {
        let result = expand_url_template(
            "https://example.com/fixed.so",
            "1.0",
            "target",
            "so",
            "name",
        )
        .unwrap();
        assert_eq!(result, "https://example.com/fixed.so");
    }

    #[test]
    fn reject_unknown_variable() {
        let err = expand_url_template("{foo}", "1", "t", "e", "name").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnknownVariable { .. }));
        if let UrlTemplateError::UnknownVariable { variable } = err {
            assert_eq!(variable, "foo");
        }
    }

    #[test]
    fn reject_unbalanced_open_brace() {
        let err = expand_url_template("prefix{var", "1", "t", "e", "name").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnbalancedBrace { .. }));
    }

    #[test]
    fn reject_unbalanced_close_brace() {
        let err = expand_url_template("prefix}", "1", "t", "e", "name").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnbalancedBrace { .. }));
    }

    #[test]
    fn reject_double_brace_escape() {
        let err = expand_url_template("{{name}}", "1", "t", "e", "name").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnbalancedBrace { .. }));
    }

    #[test]
    fn validate_template_happy() {
        assert!(validate_template("https://example.com/{version}/{target}.{ext}").is_ok());
    }

    #[test]
    fn validate_template_unknown_variable() {
        let err = validate_template("{foo}").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnknownVariable { .. }));
    }

    #[test]
    fn validate_template_unbalanced() {
        let err = validate_template("{var").unwrap_err();
        assert!(matches!(err, UrlTemplateError::UnbalancedBrace { .. }));
    }
}
