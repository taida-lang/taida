#![allow(clippy::doc_lazy_continuation)]

// N-55: Error handling conventions in this CLI binary
//
// This file uses three error handling patterns, chosen by context:
//
// 1. `expect("message")` / `unwrap()` — for invariants that indicate
//    programmer error or a fundamentally broken system (e.g. system clock
//    before epoch, Tokio runtime creation). Panic is acceptable because
//    no meaningful recovery is possible.
//
// 2. `unwrap_or` / `unwrap_or_else` — for fallible operations with safe
//    defaults (e.g. path canonicalization falling back to the original
//    path). Version resolution uses `taida::version::taida_version()`.
//
// 3. `eprintln!` + `process::exit(1)` — for user-facing errors that
//    should produce a diagnostic and terminate (e.g. missing input file,
//    parse errors, build failures). These are not panics.
//
// Library code (`src/lib.rs` and sub-modules) uses `Result<T, String>`
// for error propagation. The CLI layer in this file converts those into
// pattern 3 at the boundary.

use serde_json::json;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(feature = "community")]
use taida::auth;
#[cfg(feature = "native")]
use taida::codegen;
#[cfg(feature = "community")]
use taida::community;
use taida::diagnostics::split_diag_code_and_hint;
use taida::doc;
use taida::graph::ai_format;
use taida::graph::verify;
use taida::interpreter::Interpreter;
use taida::parser::{BuchiField, Expr, FieldDef, FuncDef, Program, Statement, parse};
use taida::pkg;
use taida::types::{CompileTarget, TypeChecker};
use taida::version::taida_version;

mod cli;
use cli::build::*;
use cli::help::*;

fn is_help_flag(raw: &str) -> bool {
    matches!(raw, "--help" | "-h")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WayFormat {
    Text,
    Json,
    Jsonl,
    Sarif,
}

impl WayFormat {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "jsonl" => Some(Self::Jsonl),
            "sarif" => Some(Self::Sarif),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WayOptions {
    format: WayFormat,
    strict: bool,
    quiet: bool,
}

impl Default for WayOptions {
    fn default() -> Self {
        Self {
            format: WayFormat::Text,
            strict: false,
            quiet: false,
        }
    }
}

fn reject_no_check_under_way() -> ! {
    eprintln!(
        "--no-check is not allowed under 'taida way'. The way hub exists to run quality checks."
    );
    std::process::exit(2);
}

fn way_should_fail(errors: usize, warnings: usize, strict: bool) -> bool {
    errors > 0 || (strict && warnings > 0)
}

fn parse_way_format_or_exit(raw: &str, command: &str) -> WayFormat {
    match WayFormat::parse(raw) {
        Some(format) => format,
        None => {
            eprintln!(
                "Unknown format '{}'. Expected: text | json | jsonl | sarif",
                raw
            );
            if command.is_empty() {
                eprintln!("Run `taida way --help` for usage.");
            } else {
                eprintln!("Run `taida way {} --help` for usage.", command);
            }
            std::process::exit(2);
        }
    }
}

fn push_way_options_args(out: &mut Vec<String>, options: WayOptions) {
    if options.strict {
        out.push("--strict".to_string());
    }
    if options.quiet {
        out.push("--quiet".to_string());
    }
    match options.format {
        WayFormat::Text => {}
        WayFormat::Json => {
            out.push("--format".to_string());
            out.push("json".to_string());
        }
        WayFormat::Jsonl => {
            out.push("--format".to_string());
            out.push("jsonl".to_string());
        }
        WayFormat::Sarif => {
            out.push("--format".to_string());
            out.push("sarif".to_string());
        }
    }
}

fn removed_command_replacement(command: &str) -> Option<&'static str> {
    match command {
        "check" => Some("taida way check"),
        "verify" => Some("taida way verify"),
        "lint" => Some("taida way lint"),
        "todo" => Some("taida way todo"),
        "inspect" => Some("taida graph summary"),
        "transpile" => Some("taida build native"),
        "compile" => Some("taida build native"),
        "deps" => Some("taida ingot deps"),
        "install" => Some("taida ingot install"),
        "update" => Some("taida ingot update"),
        "publish" => Some("taida ingot publish"),
        "cache" => Some("taida ingot cache"),
        "c" => Some("taida community"),
        _ => None,
    }
}

fn reject_removed_command(command: &str) -> ! {
    let replacement = removed_command_replacement(command).unwrap_or("taida --help");
    eprintln!(
        "[E1700] Command '{}' was removed in @e.X. Use '{}' instead.",
        command, replacement
    );
    eprintln!("        See `taida --help` for the new command set.");
    std::process::exit(2);
}

fn reject_removed_migration_command(invocation: &str) -> ! {
    eprintln!(
        "[E1700] Migration command '{}' is not available. Current CLI does not provide AST migration tooling.",
        invocation
    );
    eprintln!(
        "        Update source files manually; run `taida upgrade --help` for self-upgrade usage."
    );
    std::process::exit(2);
}

fn run_way(args: &[String], no_check: bool) {
    if no_check {
        reject_no_check_under_way();
    }

    if args.is_empty() || is_help_flag(args[0].as_str()) {
        print_way_help();
        return;
    }

    match args[0].as_str() {
        "check" => run_check_cmd(&args[1..]),
        "lint" => run_lint_cmd(&args[1..]),
        "verify" => run_verify(&args[1..]),
        "todo" => run_todo(&args[1..]),
        "migrate" => reject_removed_migration_command("taida way migrate"),
        _ => run_way_full(args),
    }
}

fn run_way_full(args: &[String]) {
    let mut options = WayOptions::default();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida way --help` for usage.");
                    std::process::exit(2);
                }
                options.format = parse_way_format_or_exit(args[i].as_str(), "");
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => options.quiet = true,
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for `taida way`: {}", raw);
                eprintln!("Run `taida way --help` for usage.");
                std::process::exit(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida way.");
                    std::process::exit(2);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let path = match path {
        Some(path) => path,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida way --help` for usage.");
            std::process::exit(2);
        }
    };

    let mut sub_args = Vec::new();
    push_way_options_args(&mut sub_args, options);
    sub_args.push(path.clone());
    run_check_cmd(&sub_args);

    let mut sub_args = Vec::new();
    push_way_options_args(&mut sub_args, options);
    sub_args.push(path.clone());
    run_lint_cmd(&sub_args);

    let mut sub_args = Vec::new();
    push_way_options_args(&mut sub_args, options);
    sub_args.push(path);
    run_verify(&sub_args);
}

fn run_ingot(args: &[String]) {
    if args.is_empty() || is_help_flag(args[0].as_str()) {
        print_ingot_help();
        return;
    }

    match args[0].as_str() {
        "deps" => run_deps(&args[1..]),
        "install" => run_install(&args[1..]),
        "migrate-lockfile" => run_migrate_lockfile(&args[1..]),
        "update" => run_update(&args[1..]),
        #[cfg(feature = "community")]
        "publish" => run_publish(&args[1..]),
        #[cfg(not(feature = "community"))]
        "publish" => {
            eprintln!("The 'taida ingot publish' command requires the 'community' feature.");
            eprintln!("Rebuild with: cargo build --features community");
            std::process::exit(1);
        }
        "cache" => run_cache(&args[1..]),
        other => {
            eprintln!("Unknown subcommand for `taida ingot`: {}", other);
            eprintln!("Run `taida ingot --help` for usage.");
            std::process::exit(2);
        }
    }
}

fn main() {
    // C25B-018: install the panic hook + fatal-signal cleanup handlers
    // **before** we otherwise perturb signal dispositions below. This
    // way a panic during very early startup (before `filtered_args`
    // parsing etc.) still runs the terminal-state-restoration path,
    // and the SIGPIPE-ignore below is unaffected (SIGPIPE is not in
    // our cleanup signal set).
    taida::panic_cleanup::install_panic_cleanup_hook();
    taida::panic_cleanup::install_signal_cleanup_handlers();

    // C22-4 / C22B-004: restore `taida <file> ... | head` as a first-class UNIX
    // pipeline. Rust binaries default to SIGPIPE-driven exit(141) the moment
    // a downstream consumer closes early; we disable that disposition here so
    // that subsequent `write(2)` calls fail with EPIPE instead — which the
    // `stdout` builtin (C22-2) silently absorbs via `writeln!+flush().ok()`.
    //
    // Scope note: this sets *process-wide* signal disposition. Matches the
    // convention of every major CLI (ripgrep, bat, fd, coreutils …). Child
    // processes started via `std::process::Command` / tokio are unaffected
    // because `execve` resets signal dispositions on the child side.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let args: Vec<String> = env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    // Check for --no-check flag
    let no_check = args.iter().any(|a| a == "--no-check");
    // Filter out --no-check from args for subcommand processing
    let filtered_args: Vec<String> = args
        .iter()
        .filter(|a| a.as_str() != "--no-check")
        .cloned()
        .collect();

    if filtered_args.len() > 1 {
        match filtered_args[1].as_str() {
            "--help" | "-h" | "help" => print_cli_help(),
            "--version" | "-V" | "version" => print_cli_version(),
            #[cfg(feature = "lsp")]
            "lsp" => run_lsp(&filtered_args[2..]),
            #[cfg(not(feature = "lsp"))]
            "lsp" => {
                eprintln!("The 'lsp' command requires the 'lsp' feature.");
                eprintln!("Rebuild with: cargo build --features lsp");
                std::process::exit(1);
            }
            old if removed_command_replacement(old).is_some() => reject_removed_command(old),
            "way" => run_way(&filtered_args[2..], no_check),
            "build" => run_build(&filtered_args[2..], no_check),
            "graph" => run_graph(&filtered_args[2..]),
            "init" => run_init(&filtered_args[2..]),
            "ingot" => run_ingot(&filtered_args[2..]),
            "doc" => run_doc(&filtered_args[2..]),
            #[cfg(feature = "community")]
            "auth" => auth::run_auth(&filtered_args[2..]),
            #[cfg(not(feature = "community"))]
            "auth" => {
                eprintln!("The 'auth' command requires the 'community' feature.");
                eprintln!("Rebuild with: cargo build --features community");
                std::process::exit(1);
            }
            #[cfg(feature = "community")]
            "community" => community::run_community(&filtered_args[2..]),
            #[cfg(not(feature = "community"))]
            "community" => {
                eprintln!("The 'community' command requires the 'community' feature.");
                eprintln!("Rebuild with: cargo build --features community");
                std::process::exit(1);
            }
            #[cfg(feature = "community")]
            "upgrade" => run_upgrade(&filtered_args[2..]),
            #[cfg(not(feature = "community"))]
            "upgrade" => {
                eprintln!("The 'upgrade' command requires the 'community' feature.");
                eprintln!("Rebuild with: cargo build --features community");
                std::process::exit(1);
            }
            _ => {
                // File execution mode
                let filename = &filtered_args[1];
                match fs::read_to_string(filename) {
                    Ok(source) => run_source(&source, filename, no_check),
                    Err(e) => {
                        eprintln!("Error reading file '{}': {}", filename, e);
                        std::process::exit(1);
                    }
                }
            }
        }
    } else {
        // REPL mode
        print_cli_version();
        println!("Type expressions to evaluate. Ctrl+D to exit.");
        println!();
        repl(no_check);
    }
}

fn run_source(source: &str, filename: &str, no_check: bool) {
    let (program, parse_errors) = parse(source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            eprintln!("{}", err);
        }
        std::process::exit(1);
    }

    // Type checking
    if !no_check {
        let mut checker = TypeChecker::new();
        checker.set_compile_target(CompileTarget::Interpreter);
        let file_path = std::path::Path::new(filename);
        if file_path.exists() {
            checker.set_source_file(file_path);
        }
        checker.check_program(&program);
        if !checker.errors.is_empty() {
            for err in &checker.errors {
                eprintln!("{}", err);
            }
            std::process::exit(1);
        }
    }

    // Gorilla ceiling warning: check for uncovered throw sites
    if !no_check {
        let findings = verify::run_check("error-coverage", &program, filename);
        for f in &findings {
            if let Some(line) = f.line {
                eprintln!("Warning: {} (line {})", f.message, line);
            } else {
                eprintln!("Warning: {}", f.message);
            }
        }
    }

    // C22-2 / C22B-002: CLI execution uses stream mode so that `stdout(...)`
    // / `debug(...)` flush to the terminal immediately. REPL (`run_repl`)
    // and in-process tests continue to use `Interpreter::new()` (buffered).
    let mut interpreter = Interpreter::new_streaming();
    // Set current file for module resolution
    if let Ok(canonical) = fs::canonicalize(filename) {
        interpreter.set_current_file(&canonical);
    } else {
        interpreter.set_current_file(Path::new(filename));
    }
    match interpreter.eval_program(&program) {
        Ok(val) => {
            // In buffered mode the Vec accumulated output during eval; drain it
            // now. In stream mode the Vec is empty (output was flushed inline),
            // so this loop is a no-op.
            if !interpreter.stream_stdout {
                for line in &interpreter.output {
                    println!("{}", line);
                }
            }
            // If the last value is not Unit and nothing was ever printed
            // via `stdout(...)`, print the value so that `taida expr.td`
            // continues to show the result of a pure-expression script.
            let no_emissions = if interpreter.stream_stdout {
                interpreter.stdout_emissions == 0
            } else {
                interpreter.output.is_empty()
            };
            if !matches!(val, taida::interpreter::Value::Unit) && no_emissions {
                println!("{}", val);
            }
        }
        Err(e) => {
            // Print any output that was collected before the error (buffered
            // mode only; in stream mode it has already been flushed inline).
            if !interpreter.stream_stdout {
                for line in &interpreter.output {
                    println!("{}", line);
                }
            }
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn run_check_cmd(args: &[String]) {
    let mut options = WayOptions::default();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_check_help();
                return;
            }
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida way check --help` for usage.");
                    std::process::exit(1);
                }
                options.format = parse_way_format_or_exit(args[i].as_str(), "check");
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => options.quiet = true,
            "--json" => {
                eprintln!("`--json` was removed. Use `taida way check --format json`.");
                std::process::exit(2);
            }
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for check: {}", raw);
                eprintln!("Run `taida way check --help` for usage.");
                std::process::exit(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida way check.");
                    std::process::exit(2);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let target = match path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida way check --help` for usage.");
            std::process::exit(2);
        }
    };

    let target_path = Path::new(&target);
    let td_files: Vec<PathBuf> = if target_path.is_dir() {
        let files = collect_td_files(target_path);
        if files.is_empty() {
            eprintln!("No .td files found in '{}'", target);
            std::process::exit(1);
        }
        files
    } else {
        vec![target_path.to_path_buf()]
    };

    let mut diagnostics: Vec<CheckDiagnostic> = Vec::new();

    for td_file in &td_files {
        let file_str = td_file.to_string_lossy().to_string();
        let source = match fs::read_to_string(td_file) {
            Ok(s) => s,
            Err(e) => {
                diagnostics.push(CheckDiagnostic {
                    stage: "io",
                    severity: "ERROR",
                    code: None,
                    message: format!("Error reading file '{}': {}", file_str, e),
                    file: Some(file_str),
                    line: None,
                    column: None,
                    suggestion: None,
                });
                continue;
            }
        };

        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            for err in &parse_errors {
                let (code, suggestion) = split_diag_code_and_hint(&err.message);
                diagnostics.push(CheckDiagnostic {
                    stage: "parse",
                    severity: "ERROR",
                    code,
                    message: err.message.clone(),
                    file: Some(file_str.clone()),
                    line: Some(err.span.line),
                    column: Some(err.span.column),
                    suggestion,
                });
            }
            continue;
        }

        let mut checker = TypeChecker::new();
        checker.set_compile_target(CompileTarget::Interpreter);
        checker.set_source_file(std::path::Path::new(&file_str));
        checker.check_program(&program);
        for err in &checker.errors {
            let (code, suggestion) = split_diag_code_and_hint(&err.message);
            diagnostics.push(CheckDiagnostic {
                stage: "type",
                severity: "ERROR",
                code,
                message: err.message.clone(),
                file: Some(file_str.clone()),
                line: Some(err.span.line),
                column: Some(err.span.column),
                suggestion,
            });
        }

        // F42 sweep [E0701]: surface direct non-tail recursion at
        // `way check` time. The verify check is registered in
        // `verify::ALL_CHECKS` for `way verify`, but `way check` does
        // not run the full verify pipeline; pulling this single check
        // in keeps PHILOSOPHY I — strict, no unbounded stacks — visible
        // during the default lint flow as well.
        let f42_findings = verify::run_check("direct-non-tail-recursion", &program, &file_str);
        for f in f42_findings {
            let (code, suggestion) = split_diag_code_and_hint(&f.message);
            diagnostics.push(CheckDiagnostic {
                stage: "verify",
                severity: "ERROR",
                code,
                message: f.message.clone(),
                file: f.file.clone(),
                line: f.line,
                column: None,
                suggestion,
            });
        }
    }

    let errors = diagnostics.iter().filter(|d| d.severity == "ERROR").count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == "WARNING")
        .count();
    let infos = diagnostics.iter().filter(|d| d.severity == "INFO").count();

    if !options.quiet {
        emit_check_diagnostics(
            &diagnostics,
            td_files.len(),
            options,
            errors,
            warnings,
            infos,
        );
    }

    if way_should_fail(errors, warnings, options.strict) {
        std::process::exit(1);
    }
}

fn emit_check_diagnostics(
    diagnostics: &[CheckDiagnostic],
    files: usize,
    options: WayOptions,
    errors: usize,
    warnings: usize,
    infos: usize,
) {
    match options.format {
        WayFormat::Text => {
            for d in diagnostics {
                match (&d.file, d.line, d.column) {
                    (Some(file), Some(line), Some(column)) => {
                        eprintln!(
                            "[{}][{}] {} ({}:{}:{})",
                            d.severity, d.stage, d.message, file, line, column
                        );
                    }
                    (Some(file), Some(line), None) => {
                        eprintln!(
                            "[{}][{}] {} ({}:{})",
                            d.severity, d.stage, d.message, file, line
                        );
                    }
                    (Some(file), None, _) => {
                        eprintln!("[{}][{}] {} ({})", d.severity, d.stage, d.message, file);
                    }
                    _ => eprintln!("[{}][{}] {}", d.severity, d.stage, d.message),
                }
            }
            eprintln!(
                "check summary: total={}, errors={}, warnings={}, info={}, files={}",
                diagnostics.len(),
                errors,
                warnings,
                infos,
                files
            );
        }
        WayFormat::Json => {
            let output = check_diagnostics_json(diagnostics, files, errors, warnings, infos);
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
        }
        WayFormat::Jsonl => {
            for d in diagnostics {
                let rec = json!({
                    "schema": "taida.diagnostic.v1",
                    "stream": "check",
                    "kind": "finding",
                    "code": d.code,
                    "message": d.message,
                    "location": {
                        "file": d.file,
                        "line": d.line,
                        "column": d.column,
                    },
                    "suggestion": d.suggestion,
                    "stage": d.stage,
                    "severity": d.severity,
                });
                println!("{}", rec);
            }
            println!(
                "{}",
                json!({
                    "schema": "taida.diagnostic.v1",
                    "stream": "check",
                    "kind": "summary",
                    "code": null,
                    "message": "check summary",
                    "location": null,
                    "suggestion": null,
                    "summary": {
                        "total": diagnostics.len(),
                        "errors": errors,
                        "warnings": warnings,
                        "info": infos,
                        "files": files,
                    }
                })
            );
        }
        WayFormat::Sarif => {
            let results: Vec<serde_json::Value> = diagnostics
                .iter()
                .map(|d| {
                    let level = match d.severity {
                        "ERROR" => "error",
                        "WARNING" => "warning",
                        _ => "note",
                    };
                    json!({
                        "ruleId": d.code.as_deref().unwrap_or(d.stage),
                        "level": level,
                        "message": { "text": d.message },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": d.file },
                                "region": {
                                    "startLine": d.line,
                                    "startColumn": d.column,
                                }
                            }
                        }]
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "version": "2.1.0",
                    "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
                    "runs": [{
                        "tool": {
                            "driver": {
                                "name": "taida way check",
                                "rules": []
                            }
                        },
                        "results": results
                    }]
                }))
                .unwrap_or_else(|_| "{}".to_string())
            );
        }
    }
}

fn check_diagnostics_json(
    diagnostics: &[CheckDiagnostic],
    files: usize,
    errors: usize,
    warnings: usize,
    infos: usize,
) -> serde_json::Value {
    json!({
        "schema": "taida.check.v1",
        "diagnostics": diagnostics
            .iter()
            .map(|d| {
                json!({
                    "stage": d.stage,
                    "severity": d.severity,
                    "code": d.code,
                    "message": d.message,
                    "location": {
                        "file": d.file,
                        "line": d.line,
                        "column": d.column,
                    },
                    "suggestion": d.suggestion,
                })
            })
            .collect::<Vec<serde_json::Value>>(),
        "summary": {
            "total": diagnostics.len(),
            "errors": errors,
            "warnings": warnings,
            "info": infos,
            "files": files,
        }
    })
}

// ── Lint subcommand ──────────────────────────

fn run_lint_cmd(args: &[String]) {
    use taida::parser::lint::lint_program_with_source;

    let mut options = WayOptions::default();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_lint_help();
                return;
            }
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida way lint --help` for usage.");
                    std::process::exit(1);
                }
                options.format = parse_way_format_or_exit(args[i].as_str(), "lint");
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => options.quiet = true,
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for lint: {}", raw);
                eprintln!("Run `taida way lint --help` for usage.");
                std::process::exit(2);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida way lint.");
                    std::process::exit(2);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let target = match path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida way lint --help` for usage.");
            std::process::exit(2);
        }
    };

    let target_path = Path::new(&target);
    let td_files: Vec<PathBuf> = if target_path.is_dir() {
        let files = collect_td_files(target_path);
        if files.is_empty() {
            eprintln!("No .td files found in '{}'", target);
            std::process::exit(2);
        }
        files
    } else {
        vec![target_path.to_path_buf()]
    };

    let mut total_diags: usize = 0;
    let mut had_parse_error: bool = false;
    let mut report = verify::VerifyReport::new();

    for td_file in &td_files {
        let file_str = td_file.to_string_lossy().to_string();
        let source = match fs::read_to_string(td_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: read error: {}", file_str, e);
                had_parse_error = true;
                continue;
            }
        };
        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            // Lint cannot run cleanly when parse errors are present.
            // Skip this file and report.
            had_parse_error = true;
            if !options.quiet {
                eprintln!(
                    "{}: parse errors prevent lint ({} error(s))",
                    file_str,
                    parse_errors.len()
                );
            }
            continue;
        }
        let diags = lint_program_with_source(&program, &source);
        total_diags += diags.len();
        for d in &diags {
            report.add(verify::VerifyFinding {
                check: "naming-convention".to_string(),
                severity: verify::Severity::Error,
                message: format!("{} {}", d.code, d.message),
                file: Some(file_str.clone()),
                line: Some(d.span.line),
            });
        }
        if !options.quiet && options.format == WayFormat::Text {
            for d in &diags {
                println!("{}", d.render(&file_str));
            }
        }
    }

    if had_parse_error {
        // Argument-level failure (lint could not clean-run somewhere)
        std::process::exit(2);
    }
    if !options.quiet {
        match options.format {
            WayFormat::Text => {}
            WayFormat::Json => println!("{}", report.format_json()),
            WayFormat::Jsonl => print!("{}", report.format_jsonl(&["naming-convention"])),
            WayFormat::Sarif => print!("{}", report.format_sarif(&["naming-convention"])),
        }
    }
    if total_diags > 0 {
        std::process::exit(1);
    }
}

// ── Compile / Transpile / Build subcommands ─────────────

#[derive(Clone, Debug)]
struct CheckDiagnostic {
    stage: &'static str,
    severity: &'static str,
    code: Option<String>,
    message: String,
    file: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    suggestion: Option<String>,
}

// ── Upgrade subcommand ──────────────────────────────────────

#[cfg(feature = "community")]
fn run_upgrade(args: &[String]) {
    use taida::upgrade::{UpgradeConfig, VersionFilter};

    if args.len() == 1 && is_help_flag(args[0].as_str()) {
        print_upgrade_help();
        return;
    }

    if args.iter().any(|a| a == "--d28") {
        reject_removed_migration_command("taida upgrade --d28");
    }
    if args.iter().any(|a| a == "--d29") {
        reject_removed_migration_command("taida upgrade --d29");
    }
    if args.iter().any(|a| a == "--e30") {
        reject_removed_migration_command("taida upgrade --e30");
    }

    let mut check_only = false;
    let mut generation: Option<String> = None;
    let mut label: Option<String> = None;
    let mut exact: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_upgrade_help();
                return;
            }
            "--check" => {
                check_only = true;
            }
            "--gen" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --gen requires a value");
                    std::process::exit(1);
                }
                generation = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --label requires a value");
                    std::process::exit(1);
                }
                label = Some(args[i].clone());
            }
            "--version" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --version requires a value");
                    std::process::exit(1);
                }
                exact = Some(args[i].clone());
            }
            other => {
                eprintln!("Error: unknown option '{}'", other);
                eprintln!("Run `taida upgrade --help` for usage.");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Validate mutual exclusivity
    if exact.is_some() && (generation.is_some() || label.is_some()) {
        eprintln!("Error: --version cannot be combined with --gen or --label");
        std::process::exit(1);
    }

    let config = UpgradeConfig {
        check_only,
        filter: VersionFilter {
            generation,
            label,
            exact,
        },
    };

    if let Err(e) = taida::upgrade::run(config) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn collect_release_gate_sites_for_files(td_files: &[PathBuf]) -> Vec<TodoStubSite> {
    let mut sites = Vec::<TodoStubSite>::new();
    for td_file in td_files {
        sites.extend(scan_release_gate_sites(td_file));
    }
    sites.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.kind.cmp(b.kind))
    });
    sites.dedup_by(|a, b| {
        a.file == b.file && a.line == b.line && a.column == b.column && a.kind == b.kind
    });
    sites
}

fn collect_td_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_td_files(&path));
            } else if path.extension().is_some_and(|e| e == "td") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[derive(Debug, Clone)]
struct TodoItem {
    id: Option<String>,
    task: Option<String>,
    file: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone)]
struct TodoStubSite {
    kind: &'static str,
    file: String,
    line: usize,
    column: usize,
}

#[derive(Default)]
struct TodoScanResult {
    todos: Vec<TodoItem>,
    sites: Vec<TodoStubSite>,
}

fn extract_string_field(fields: &[BuchiField], name: &str) -> Option<String> {
    fields.iter().find_map(|f| {
        if f.name == name {
            if let Expr::StringLit(s, _) = &f.value {
                Some(s.clone())
            } else {
                None
            }
        } else {
            None
        }
    })
}

fn scan_field_defaults(field: &FieldDef, file: &str, out: &mut TodoScanResult) {
    if let Some(default_expr) = &field.default_value {
        scan_expr_for_todo(default_expr, file, out);
    }
    if let Some(method) = &field.method_def {
        scan_func_for_todo(method, file, out);
    }
}

fn scan_func_for_todo(func: &FuncDef, file: &str, out: &mut TodoScanResult) {
    for param in &func.params {
        if let Some(default_expr) = &param.default_value {
            scan_expr_for_todo(default_expr, file, out);
        }
    }
    for stmt in &func.body {
        scan_stmt_for_todo(stmt, file, out);
    }
}

fn scan_stmt_for_todo(stmt: &Statement, file: &str, out: &mut TodoScanResult) {
    match stmt {
        Statement::Expr(expr) => scan_expr_for_todo(expr, file, out),
        Statement::EnumDef(_) => {}
        // (E30 Sub-step 2.1) ClassLikeDef + kind dispatch (旧 TypeDef/MoldDef/InheritanceDef を統合)
        Statement::ClassLikeDef(cl) => {
            for field in &cl.fields {
                scan_field_defaults(field, file, out);
            }
        }
        Statement::FuncDef(fd) => scan_func_for_todo(fd, file, out),
        Statement::Assignment(assign) => scan_expr_for_todo(&assign.value, file, out),
        Statement::ErrorCeiling(ec) => {
            for stmt in &ec.handler_body {
                scan_stmt_for_todo(stmt, file, out);
            }
        }
        Statement::UnmoldForward(uf) => scan_expr_for_todo(&uf.source, file, out),
        Statement::UnmoldBackward(ub) => scan_expr_for_todo(&ub.source, file, out),
        Statement::Import(_) | Statement::Export(_) => {}
    }
}

fn scan_expr_for_todo(expr: &Expr, file: &str, out: &mut TodoScanResult) {
    match expr {
        Expr::MoldInst(name, type_args, fields, span) => {
            if name == "TODO" {
                out.todos.push(TodoItem {
                    id: extract_string_field(fields, "id"),
                    task: extract_string_field(fields, "task"),
                    file: file.to_string(),
                    line: span.line,
                    column: span.column,
                });
                out.sites.push(TodoStubSite {
                    kind: "TODO",
                    file: file.to_string(),
                    line: span.line,
                    column: span.column,
                });
            } else if name == "Stub" {
                out.sites.push(TodoStubSite {
                    kind: "Stub",
                    file: file.to_string(),
                    line: span.line,
                    column: span.column,
                });
            }
            for arg in type_args {
                scan_expr_for_todo(arg, file, out);
            }
            for field in fields {
                scan_expr_for_todo(&field.value, file, out);
            }
        }
        Expr::BuchiPack(fields, _) | Expr::TypeInst(_, fields, _) => {
            for field in fields {
                scan_expr_for_todo(&field.value, file, out);
            }
        }
        Expr::ListLit(items, _) | Expr::Pipeline(items, _) => {
            for item in items {
                scan_expr_for_todo(item, file, out);
            }
        }
        Expr::BinaryOp(left, _, right, _) => {
            scan_expr_for_todo(left, file, out);
            scan_expr_for_todo(right, file, out);
        }
        Expr::UnaryOp(_, inner, _) | Expr::Unmold(inner, _) | Expr::Throw(inner, _) => {
            scan_expr_for_todo(inner, file, out);
        }
        Expr::FuncCall(callee, args, _) => {
            scan_expr_for_todo(callee, file, out);
            for arg in args {
                scan_expr_for_todo(arg, file, out);
            }
        }
        Expr::MethodCall(receiver, _, args, _) => {
            scan_expr_for_todo(receiver, file, out);
            for arg in args {
                scan_expr_for_todo(arg, file, out);
            }
        }
        Expr::FieldAccess(receiver, _, _) => scan_expr_for_todo(receiver, file, out),
        Expr::CondBranch(arms, _) => {
            for arm in arms {
                if let Some(cond) = &arm.condition {
                    scan_expr_for_todo(cond, file, out);
                }
                for stmt in &arm.body {
                    if let Statement::Expr(e) = stmt {
                        scan_expr_for_todo(e, file, out);
                    }
                }
            }
        }
        Expr::Lambda(params, body, _) => {
            for param in params {
                if let Some(default_expr) = &param.default_value {
                    scan_expr_for_todo(default_expr, file, out);
                }
            }
            scan_expr_for_todo(body, file, out);
        }
        Expr::IntLit(_, _)
        | Expr::FloatLit(_, _)
        | Expr::StringLit(_, _)
        | Expr::TemplateLit(_, _)
        | Expr::BoolLit(_, _)
        | Expr::Gorilla(_)
        | Expr::Ident(_, _)
        | Expr::EnumVariant(_, _, _)
        | Expr::TypeLiteral(_, _, _)
        | Expr::Placeholder(_)
        | Expr::Hole(_) => {}
    }
}

fn scan_program_for_todo(program: &Program, file: &Path) -> TodoScanResult {
    let file_label = file.to_string_lossy().to_string();
    let mut out = TodoScanResult::default();
    for stmt in &program.statements {
        scan_stmt_for_todo(stmt, &file_label, &mut out);
    }
    out
}

fn resolve_local_import_path(base_dir: &Path, import_path: &str) -> PathBuf {
    base_dir.join(import_path)
}

fn collect_release_scan_files(target_path: &Path) -> Vec<PathBuf> {
    if target_path.is_dir() {
        return collect_td_files(target_path);
    }

    let mut out = Vec::<PathBuf>::new();
    let mut pending = vec![
        target_path
            .canonicalize()
            .unwrap_or_else(|_| target_path.to_path_buf()),
    ];
    let mut seen = std::collections::HashSet::<PathBuf>::new();

    while let Some(file) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        out.push(file.clone());

        let source = match fs::read_to_string(&file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            continue;
        }

        // N-48: parent() returns None only for root or prefix-less paths,
        // which cannot occur for resolved .td file paths. "." is a safe default.
        let base_dir = file.parent().unwrap_or(Path::new("."));
        for stmt in &program.statements {
            if let Statement::Import(import) = stmt {
                let dep = resolve_local_import_path(base_dir, &import.path);
                if dep.exists() && dep.is_file() {
                    // Canonicalize for dedup; fall back to the joined path if
                    // the file system rejects it (e.g. dangling symlink).
                    pending.push(dep.canonicalize().unwrap_or(dep));
                }
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

fn scan_release_gate_sites(target_path: &Path) -> Vec<TodoStubSite> {
    let mut sites = Vec::<TodoStubSite>::new();
    for td_file in collect_release_scan_files(target_path) {
        let source = match fs::read_to_string(&td_file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            continue;
        }
        let scan = scan_program_for_todo(&program, &td_file);
        sites.extend(scan.sites);
    }
    sites
}

fn report_release_gate_violations(
    mut sites: Vec<TodoStubSite>,
    diag_format: DiagFormat,
    compile_stats: &mut CompileDiagStats,
) {
    if sites.is_empty() {
        return;
    }
    sites.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.kind.cmp(b.kind))
    });
    if diag_format == DiagFormat::Jsonl {
        emit_compile_diag_jsonl(
            compile_stats,
            "ERROR",
            "verify",
            None,
            "Release gate failed: TODO/Stub remains in source.",
            None,
            None,
            None,
            Some("Remove all TODO/Stub molds before --release build.".to_string()),
        );
        for site in &sites {
            emit_compile_diag_jsonl(
                compile_stats,
                "ERROR",
                "verify",
                None,
                &format!("{} remains in source", site.kind),
                Some(&site.file),
                Some(site.line),
                Some(site.column),
                Some("Replace TODO/Stub with concrete implementation.".to_string()),
            );
        }
    } else {
        eprintln!("Release gate failed: TODO/Stub remains in source.");
        for site in sites.iter().take(20) {
            eprintln!(
                "  - {} at {}:{}:{}",
                site.kind, site.file, site.line, site.column
            );
        }
        if sites.len() > 20 {
            eprintln!("  ... and {} more site(s)", sites.len() - 20);
        }
    }
}

fn run_todo(args: &[String]) {
    let mut options = WayOptions::default();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_todo_help();
                return;
            }
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida way todo --help` for usage.");
                    std::process::exit(1);
                }
                options.format = parse_way_format_or_exit(args[i].as_str(), "todo");
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => options.quiet = true,
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for todo: {}", raw);
                eprintln!("Run `taida way todo --help` for usage.");
                std::process::exit(1);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one [PATH] is accepted for taida way todo.");
                    std::process::exit(1);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let target = path.unwrap_or_else(|| ".".to_string());
    let target_path = Path::new(&target);
    let td_files: Vec<PathBuf> = if target_path.is_dir() {
        let files = collect_td_files(target_path);
        if files.is_empty() {
            eprintln!("No .td files found in '{}'", target);
            std::process::exit(1);
        }
        files
    } else {
        vec![target_path.to_path_buf()]
    };

    let mut merged = TodoScanResult::default();
    for td_file in &td_files {
        let source = match fs::read_to_string(td_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", td_file.display(), e);
                std::process::exit(1);
            }
        };
        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            for err in &parse_errors {
                eprintln!("{}: {}", td_file.display(), err);
            }
            std::process::exit(1);
        }
        let scan = scan_program_for_todo(&program, td_file);
        merged.todos.extend(scan.todos);
    }

    merged.todos.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.id.cmp(&b.id))
    });

    let mut by_id = std::collections::BTreeMap::<Option<String>, usize>::new();
    let mut by_file = std::collections::BTreeMap::<String, usize>::new();
    for todo in &merged.todos {
        *by_id.entry(todo.id.clone()).or_insert(0) += 1;
        *by_file.entry(todo.file.clone()).or_insert(0) += 1;
    }

    if options.quiet {
        return;
    }

    let todos_json: Vec<serde_json::Value> = merged
        .todos
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "task": t.task,
                "file": t.file,
                "line": t.line,
                "column": t.column,
            })
        })
        .collect();
    let by_id_json: Vec<serde_json::Value> = by_id
        .iter()
        .map(|(id, count)| json!({ "id": id, "count": count }))
        .collect();
    let by_file_json: Vec<serde_json::Value> = by_file
        .iter()
        .map(|(file, count)| json!({ "file": file, "count": count }))
        .collect();
    let output = json!({
        "total": merged.todos.len(),
        "todos": todos_json,
        "byId": by_id_json,
        "byFile": by_file_json,
    });

    match options.format {
        WayFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
            );
            return;
        }
        WayFormat::Jsonl => {
            for todo in &merged.todos {
                println!(
                    "{}",
                    json!({
                        "schema": "taida.diagnostic.v1",
                        "stream": "todo",
                        "kind": "finding",
                        "code": null,
                        "message": todo.task,
                        "location": {
                            "file": todo.file,
                            "line": todo.line,
                            "column": todo.column,
                        },
                        "suggestion": null,
                        "severity": "INFO",
                        "id": todo.id,
                    })
                );
            }
            println!(
                "{}",
                json!({
                    "schema": "taida.diagnostic.v1",
                    "stream": "todo",
                    "kind": "summary",
                    "code": null,
                    "message": "todo summary",
                    "location": null,
                    "suggestion": null,
                    "summary": {
                        "total": merged.todos.len(),
                        "errors": 0,
                        "warnings": 0,
                        "info": merged.todos.len(),
                    }
                })
            );
            return;
        }
        WayFormat::Sarif => {
            let results: Vec<serde_json::Value> = merged
                .todos
                .iter()
                .map(|todo| {
                    json!({
                        "ruleId": "todo",
                        "level": "note",
                        "message": { "text": todo.task },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": todo.file },
                                "region": {
                                    "startLine": todo.line,
                                    "startColumn": todo.column,
                                }
                            }
                        }]
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "version": "2.1.0",
                    "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
                    "runs": [{
                        "tool": {
                            "driver": {
                                "name": "taida way todo",
                                "rules": []
                            }
                        },
                        "results": results
                    }]
                }))
                .unwrap_or_else(|_| "{}".to_string())
            );
            return;
        }
        WayFormat::Text => {}
    }

    if merged.todos.is_empty() {
        println!("No TODO molds found.");
        return;
    }

    println!("TODOs: {}", merged.todos.len());
    for todo in &merged.todos {
        let id = todo.id.as_deref().unwrap_or("<missing-id>");
        let task = todo.task.as_deref().unwrap_or("<missing-task>");
        println!(
            "- [{}] {}:{}:{} {}",
            id, todo.file, todo.line, todo.column, task
        );
    }
    println!();
    println!("By ID:");
    for (id, count) in by_id {
        println!(
            "  - {}: {}",
            id.unwrap_or_else(|| "<missing-id>".to_string()),
            count
        );
    }
    println!("By File:");
    for (file, count) in by_file {
        println!("  - {}: {}", file, count);
    }
}

// ── Graph subcommand ────────────────────────────────────

fn run_graph(args: &[String]) {
    if args.first().is_some_and(|arg| arg == "summary") {
        run_graph_summary(&args[1..]);
        return;
    }

    let mut path: Option<String> = None;
    let mut output_path: Option<String> = None;
    let mut recursive = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_graph_help();
                return;
            }
            "--recursive" | "-r" => {
                recursive = true;
            }
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for -o/--output.");
                    eprintln!("Run `taida graph --help` for usage.");
                    std::process::exit(1);
                }
                output_path = Some(args[i].clone());
            }
            _ => {
                if args[i].starts_with('-') {
                    eprintln!(
                        "Unknown option '{}'. Run `taida graph --help` for usage.",
                        args[i]
                    );
                    std::process::exit(1);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let file_path = match path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida graph --help` for usage.");
            std::process::exit(1);
        }
    };

    let output = if recursive {
        match ai_format::format_ai_json_recursive(&file_path) {
            Ok(json) => json,
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    } else {
        let source = match fs::read_to_string(&file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", file_path, e);
                std::process::exit(1);
            }
        };

        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            for err in &parse_errors {
                eprintln!("{}", err);
            }
            std::process::exit(1);
        }

        ai_format::format_ai_json(&program, &file_path)
    };

    if let Some(out_path) = &output_path {
        let out = Path::new(out_path);
        let resolved = if out.parent().is_none_or(|p| p.as_os_str().is_empty()) {
            let graph_dir = find_packages_tdm()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".taida")
                .join("graph");
            if let Err(e) = fs::create_dir_all(&graph_dir) {
                eprintln!(
                    "Error creating graph directory '{}': {}",
                    graph_dir.display(),
                    e
                );
                std::process::exit(1);
            }
            graph_dir.join(out)
        } else {
            out.to_path_buf()
        };
        match fs::write(&resolved, &output) {
            Ok(_) => println!("Graph written to {}", resolved.display()),
            Err(e) => {
                eprintln!("Error writing graph to '{}': {}", resolved.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        print!("{}", output);
    }
}

fn run_graph_summary(args: &[String]) {
    let mut format_type = "text".to_string();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_graph_summary_help();
                return;
            }
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida graph summary --help` for usage.");
                    std::process::exit(1);
                }
                match args[i].as_str() {
                    "text" | "json" | "sarif" => {
                        format_type = args[i].clone();
                    }
                    other => {
                        eprintln!("Unknown format '{}'. Expected: text | json | sarif", other);
                        std::process::exit(1);
                    }
                }
            }
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for graph summary: {}", raw);
                eprintln!("Run `taida graph summary --help` for usage.");
                std::process::exit(1);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida graph summary.");
                    std::process::exit(1);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let file_path = match path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida graph summary --help` for usage.");
            std::process::exit(1);
        }
    };

    let source = match fs::read_to_string(&file_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading file '{}': {}", file_path, e);
            std::process::exit(1);
        }
    };

    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            eprintln!("{}", err);
        }
        std::process::exit(1);
    }

    let summary = verify::structural_summary(&program, &file_path);
    match format_type.as_str() {
        "sarif" => print!("{}", format_graph_summary_sarif(&summary)),
        _ => println!("{}", summary),
    }
}

fn format_graph_summary_sarif(summary_json: &str) -> String {
    let summary =
        serde_json::from_str::<serde_json::Value>(summary_json).unwrap_or_else(|_| json!({}));
    serde_json::to_string_pretty(&json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [
            {
                "tool": {
                    "driver": {
                        "name": "taida-graph-summary",
                        "version": taida_version(),
                        "rules": []
                    }
                },
                "results": [],
                "properties": {
                    "summary": summary
                }
            }
        ]
    }))
    .expect("graph summary SARIF serialization should not fail")
}

// ── Verify subcommand ───────────────────────────────────

fn run_verify(args: &[String]) {
    let mut checks: Vec<String> = Vec::new();
    let mut options = WayOptions::default();
    let mut path: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_verify_help();
                return;
            }
            "--check" | "-c" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --check.");
                    eprintln!("Run `taida way verify --help` for usage.");
                    std::process::exit(1);
                }
                if !verify::ALL_CHECKS.contains(&args[i].as_str()) {
                    eprintln!(
                        "Unknown check '{}'. Available checks: {}",
                        args[i],
                        verify::ALL_CHECKS.join(", ")
                    );
                    std::process::exit(1);
                }
                checks.push(args[i].clone());
            }
            "--format" | "-f" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --format.");
                    eprintln!("Run `taida way verify --help` for usage.");
                    std::process::exit(1);
                }
                options.format = parse_way_format_or_exit(args[i].as_str(), "verify");
            }
            "--strict" => options.strict = true,
            "--quiet" | "-q" => options.quiet = true,
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for verify: {}", raw);
                eprintln!("Run `taida way verify --help` for usage.");
                std::process::exit(1);
            }
            _ => {
                if path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida way verify.");
                    std::process::exit(1);
                }
                path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let file_path = match path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida way verify --help` for usage.");
            std::process::exit(1);
        }
    };

    // Collect target files: directory -> recursive .td collection, file -> single file
    let target_path = Path::new(&file_path);
    let td_files: Vec<PathBuf> = if target_path.is_dir() {
        let files = collect_td_files(target_path);
        if files.is_empty() {
            eprintln!("No .td files found in '{}'", file_path);
            std::process::exit(1);
        }
        files
    } else {
        vec![target_path.to_path_buf()]
    };

    // Aggregate report across all files
    let mut report = verify::VerifyReport::new();

    for td_file in &td_files {
        let file_str = td_file.to_string_lossy().to_string();
        let source = match fs::read_to_string(td_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", file_str, e);
                std::process::exit(1);
            }
        };

        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            for err in &parse_errors {
                eprintln!("{}: {}", file_str, err);
            }
            std::process::exit(1);
        }

        if checks.is_empty() {
            let file_report = verify::run_all_checks(&program, &file_str);
            for f in file_report.findings {
                report.add(f);
            }
        } else {
            for check in &checks {
                let findings = verify::run_check(check, &program, &file_str);
                for f in findings {
                    report.add(f);
                }
            }
        }
    }

    let checks_ref: Vec<&str> = if checks.is_empty() {
        verify::ALL_CHECKS.to_vec()
    } else {
        checks.iter().map(|s| s.as_str()).collect()
    };
    if !options.quiet {
        let output = match options.format {
            WayFormat::Json => report.format_json(),
            WayFormat::Jsonl => report.format_jsonl(&checks_ref),
            WayFormat::Sarif => report.format_sarif(&checks_ref),
            WayFormat::Text => report.format_text(&checks_ref),
        };
        print!("{}", output);
    }

    if way_should_fail(report.errors(), report.warnings(), options.strict) {
        std::process::exit(1);
    }
}

// ── Init subcommand ──────────────────────────────────────

fn run_init(args: &[String]) {
    // ── CLI parsing (RC2.6-3c) ──────────────────────────
    //
    // Accepted forms:
    //   taida init                           → SourceOnly in "."
    //   taida init <dir>                     → SourceOnly in <dir>
    //   taida init --target rust-addon       → RustAddon in "."
    //   taida init --target rust-addon <dir> → RustAddon in <dir>
    //   taida init <dir> --target rust-addon → RustAddon in <dir>
    //   taida init --help / -h               → help text
    let mut target = pkg::init::InitTarget::SourceOnly;
    let mut dir_arg: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_init_help();
                return;
            }
            "--target" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --target.");
                    eprintln!("Run `taida init --help` for usage.");
                    std::process::exit(1);
                }
                match args[i].as_str() {
                    "rust-addon" => target = pkg::init::InitTarget::RustAddon,
                    other => {
                        eprintln!("Unknown init target '{}'. Supported: rust-addon", other);
                        eprintln!("Run `taida init --help` for usage.");
                        std::process::exit(1);
                    }
                }
            }
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for init: {}", raw);
                eprintln!("Run `taida init --help` for usage.");
                std::process::exit(1);
            }
            positional => {
                if dir_arg.is_some() {
                    eprintln!("Too many arguments.");
                    eprintln!("Run `taida init --help` for usage.");
                    std::process::exit(1);
                }
                dir_arg = Some(positional.to_string());
            }
        }
        i += 1;
    }

    let dir_name = dir_arg.as_deref().unwrap_or(".");
    let dir = Path::new(dir_name);

    // Determine project name from directory name
    let project_name = if dir_name == "." {
        env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "my-project".to_string())
    } else {
        Path::new(dir_name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir_name.to_string())
    };

    // Create directory if needed
    if dir_name != "."
        && let Err(e) = fs::create_dir_all(dir)
    {
        eprintln!("Error creating directory '{}': {}", dir_name, e);
        std::process::exit(1);
    }

    // Delegate to pkg::init::init_project (RC2.6-3a)
    match pkg::init::init_project(dir, &project_name, target) {
        Ok(created) => {
            let target_label = match target {
                pkg::init::InitTarget::RustAddon => " (rust-addon)",
                pkg::init::InitTarget::SourceOnly => "",
            };
            println!(
                "Initialized Taida project '{}'{} in {}",
                project_name,
                target_label,
                dir.display()
            );
            for file in &created {
                println!("  {}", file);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

// ── Deps subcommand ──────────────────────────────────────

fn run_deps(args: &[String]) {
    match args {
        [] => {}
        [arg] if is_help_flag(arg.as_str()) => {
            print_deps_help();
            return;
        }
        _ => {
            eprintln!("Unexpected arguments.");
            eprintln!("Run `taida ingot deps --help` for usage.");
            std::process::exit(1);
        }
    }

    // Find project root by looking for packages.tdm
    let project_dir = find_packages_tdm().unwrap_or_else(|| {
        eprintln!("No packages.tdm found in current directory or parent directories.");
        eprintln!("Run 'taida init' to create a new project.");
        std::process::exit(1);
    });

    // Parse manifest
    let manifest = match pkg::manifest::Manifest::from_dir(&project_dir) {
        Ok(Some(m)) => m,
        Ok(None) => {
            eprintln!("No packages.tdm found in '{}'", project_dir.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error parsing packages.tdm: {}", e);
            std::process::exit(1);
        }
    };

    if manifest.deps.is_empty() {
        println!("No dependencies defined in packages.tdm");
        return;
    }

    println!("Resolving dependencies for '{}'...", manifest.name);

    // Resolve dependencies using provider chain
    let result = pkg::resolver::resolve_deps(&manifest);

    // Report errors
    for err in &result.errors {
        eprintln!("  ERROR: {}", err);
    }

    // Strict mode for `taida ingot deps`: never install or write lockfile on resolve errors.
    if !result.errors.is_empty() {
        eprintln!("Dependency resolution failed. Skipping install and lockfile update.");
        std::process::exit(1);
    }

    // Install resolved dependencies
    if !result.resolved.is_empty() {
        match pkg::resolver::install_deps(&manifest, &result) {
            Ok(()) => {
                for (name, path) in &result.resolved {
                    println!("  {} -> {}", name, path.display());
                }
                println!(
                    "Installed {} dependency(ies) in .taida/deps/",
                    result.resolved.len()
                );
            }
            Err(e) => {
                eprintln!("Error installing dependencies: {}", e);
                std::process::exit(1);
            }
        }

        // Generate lockfile
        match pkg::resolver::write_lockfile(&manifest, &result) {
            Ok(()) => println!("Updated taida.lock"),
            Err(e) => eprintln!("Warning: could not write lockfile: {}", e),
        }
    }
}

// ── Install subcommand ──────────────────────────────────

fn run_install(args: &[String]) {
    // RC1.5-3c: parse --force-refresh flag
    // RC2.7-4a: parse --allow-local-addon-build flag
    // C17-2: parse --no-remote-check (mutually exclusive with --force-refresh)
    let mut force_refresh = false;
    let mut no_remote_check = false;
    let mut allow_local_addon_build = false;
    let mut allow_fresh = false;
    let mut frozen = false;
    let mut filtered: Vec<&str> = Vec::new();
    for arg in args {
        if arg == "--force-refresh" {
            force_refresh = true;
        } else if arg == "--no-remote-check" {
            no_remote_check = true;
        } else if arg == "--allow-local-addon-build" {
            allow_local_addon_build = true;
        } else if arg == "--allow-fresh" {
            allow_fresh = true;
        } else if arg == "--frozen" {
            frozen = true;
        } else if is_help_flag(arg.as_str()) {
            print_install_help();
            return;
        } else {
            filtered.push(arg.as_str());
        }
    }
    if !filtered.is_empty() {
        eprintln!("Unexpected arguments.");
        eprintln!("Run `taida ingot install --help` for usage.");
        std::process::exit(1);
    }
    // C17-2: mutual exclusion is a hard error so users cannot silently
    // combine the two refresh knobs with surprising semantics.
    let refresh_flags = pkg::resolver::StoreRefreshFlags {
        force_refresh,
        no_remote_check,
    };
    if let Err(msg) = refresh_flags.validate() {
        eprintln!("Error: {}", msg);
        eprintln!("Run `taida ingot install --help` for usage.");
        std::process::exit(1);
    }

    // Find project root by looking for packages.tdm
    let project_dir = find_packages_tdm().unwrap_or_else(|| {
        eprintln!("No packages.tdm found in current directory or parent directories.");
        eprintln!("Run 'taida init' to create a new project.");
        std::process::exit(1);
    });

    // Parse manifest
    let manifest = match pkg::manifest::Manifest::from_dir(&project_dir) {
        Ok(Some(m)) => m,
        Ok(None) => {
            eprintln!("No packages.tdm found in '{}'", project_dir.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error parsing packages.tdm: {}", e);
            std::process::exit(1);
        }
    };

    if manifest.deps.is_empty() {
        println!("No dependencies defined in packages.tdm");
        let lock_path = project_dir.join(".taida").join("taida.lock");
        if frozen {
            match pkg::lockfile::Lockfile::read(&lock_path) {
                Ok(Some(lockfile)) if lockfile.is_up_to_date(&[]) => {
                    println!("taida.lock is frozen and up to date");
                    return;
                }
                Ok(Some(_)) => {
                    eprintln!(
                        "[E32K2_LOCKFILE_DRIFT] --frozen requires .taida/taida.lock to match packages.tdm"
                    );
                    std::process::exit(1);
                }
                Ok(None) => {
                    eprintln!(
                        "[E32K2_LOCKFILE_DRIFT] --frozen requires existing .taida/taida.lock"
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
            }
        }
        println!("Generated taida.lock (empty)");
        // Write empty lockfile
        let lockfile = pkg::lockfile::Lockfile::from_resolved(&[]);
        if let Some(parent) = lock_path.parent() {
            // N-56: directory creation error is caught by the subsequent
            // lockfile.write() call, which will report a clear error.
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = lockfile.write(&lock_path) {
            eprintln!("Warning: could not write lockfile: {}", e);
        }
        return;
    }

    println!("Installing dependencies for '{}'...", manifest.name);

    // Read existing lockfile to pin generation-only versions for reproducibility
    let lock_path = project_dir.join(".taida").join("taida.lock");
    let existing_lockfile = match pkg::lockfile::Lockfile::read(&lock_path) {
        Ok(lockfile) => lockfile,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    if frozen && existing_lockfile.is_none() {
        eprintln!("[E32K2_LOCKFILE_DRIFT] --frozen requires existing .taida/taida.lock");
        std::process::exit(1);
    }

    // Resolve all dependencies using the provider chain,
    // pinning generation-only versions to locked exact versions when available.
    // C17-2: forward refresh flags so the StoreProvider can consult the
    // stale-detection decision table (or bypass it for --force-refresh).
    let result = match &existing_lockfile {
        Some(lf) => pkg::resolver::resolve_deps_locked_with_flags(&manifest, lf, refresh_flags),
        None => pkg::resolver::resolve_deps_with_flags(&manifest, refresh_flags),
    };

    // Report errors
    for err in &result.errors {
        eprintln!("  ERROR: {}", err);
    }

    if result.errors.is_empty() {
        // Triple equality (version / source / integrity) is only required
        // under --frozen. Non-frozen install is documented as "generate /
        // update the lockfile", so legitimate drift (version bump in
        // packages.tdm, newly added dep) must rewrite the lockfile rather
        // than fail. Schema malformation is independently caught by
        // `Lockfile::read` -> `parse` ->
        // `validate_schema`, so it remains rejected regardless of frozen.
        if frozen {
            if let Some(lockfile) = &existing_lockfile
                && let Err(e) = lockfile.validate_resolved_bindings(&result.packages)
            {
                eprintln!("{}", e);
                std::process::exit(1);
            }
            if !existing_lockfile
                .as_ref()
                .map(|lf| lf.is_up_to_date(&result.packages))
                .unwrap_or(false)
            {
                eprintln!(
                    "[E32K2_LOCKFILE_DRIFT] --frozen requires .taida/taida.lock to match packages.tdm"
                );
                std::process::exit(1);
            }
        }
    }
    if frozen && !result.errors.is_empty() {
        eprintln!("\nSome dependencies could not be resolved. See errors above.");
        std::process::exit(1);
    }

    // Install resolved dependencies
    let mut addon_map: std::collections::BTreeMap<String, pkg::lockfile::LockedAddon> =
        std::collections::BTreeMap::new();
    if !result.resolved.is_empty() {
        match pkg::resolver::install_deps(&manifest, &result) {
            Ok(()) => {
                for pkg in &result.packages {
                    let source_label = match &pkg.source {
                        pkg::provider::PackageSource::Path(p) => format!("path:{}", p),
                        pkg::provider::PackageSource::CoreBundled => "bundled".to_string(),
                        pkg::provider::PackageSource::Store { org, name } => {
                            format!("github:{}/{}", org, name)
                        }
                    };
                    println!("  {} @{} ({})", pkg.name, pkg.version, source_label);
                }
                println!(
                    "Installed {} package(s) in .taida/deps/",
                    result.packages.len()
                );
            }
            Err(e) => {
                eprintln!("Error installing dependencies: {}", e);
                std::process::exit(1);
            }
        }

        // RC1.5-3a: install addon prebuilds
        // RC2.7-4b: pass allow_local_addon_build for fallback policy
        let existing_lock = pkg::lockfile::Lockfile::read(&lock_path).unwrap_or(None);
        addon_map = match pkg::resolver::install_addon_prebuilds(
            &manifest,
            &result,
            force_refresh,
            existing_lock.as_ref(),
            allow_local_addon_build,
            pkg::resolver::AddonInstallPolicy::from_manifest(&manifest, allow_fresh),
        ) {
            Ok(map) => map,
            Err(e) => {
                eprintln!("Error installing addon prebuilds: {}", e);
                std::process::exit(1);
            }
        };

        if !addon_map.is_empty() {
            for (pkg_name, addon) in &addon_map {
                println!("  Addon {} @ {} ({})", pkg_name, addon.target, addon.sha256);
            }
        }
    }

    if frozen {
        println!("taida.lock is frozen and up to date");
    } else {
        // Generate lockfile (always, even if some deps failed)
        // RC1.5: include addon info if addon prebuilds were installed
        match pkg::resolver::write_lockfile_with_addons(&manifest, &result, &addon_map) {
            Ok(()) => println!("Generated taida.lock"),
            Err(e) => eprintln!("Warning: could not write lockfile: {}", e),
        }
    }

    if !result.errors.is_empty() {
        eprintln!("\nSome dependencies could not be resolved. See errors above.");
        std::process::exit(1);
    }
}

fn run_migrate_lockfile(args: &[String]) {
    for arg in args {
        if is_help_flag(arg.as_str()) {
            println!(
                "\
Usage:
  taida ingot migrate-lockfile

Behavior:
  Rewrite `.taida/taida.lock` from schema v1 / `fnv1a:` integrity to
  the current schema / `sha256:` integrity using the installed `.taida/deps` tree.
  Missing installed dependencies fail with `[E32K2_LOCKFILE_MIGRATE_FAIL]`."
            );
            return;
        }
    }
    if !args.is_empty() {
        eprintln!("Unexpected arguments.");
        eprintln!("Run `taida ingot migrate-lockfile --help` for usage.");
        std::process::exit(1);
    }

    let project_dir = find_packages_tdm().unwrap_or_else(|| {
        eprintln!("No packages.tdm found in current directory or parent directories.");
        eprintln!("Run 'taida init' to create a new project.");
        std::process::exit(1);
    });
    let lock_path = project_dir.join(".taida").join("taida.lock");
    let deps_dir = project_dir.join(".taida").join("deps");

    match pkg::lockfile::Lockfile::migrate_current_from_installed(&lock_path, &deps_dir) {
        Ok(lockfile) => {
            println!(
                "Migrated taida.lock to schema v{} ({} package(s))",
                lockfile.version,
                lockfile.packages.len()
            );
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

// ── Update subcommand ──────────────────────────────────

fn run_update(args: &[String]) {
    // Parse --allow-local-addon-build for local addon development.
    let mut allow_local_addon_build = false;
    for arg in args {
        if arg == "--allow-local-addon-build" {
            allow_local_addon_build = true;
        } else if is_help_flag(arg.as_str()) {
            print_update_help();
            return;
        } else {
            eprintln!("Unexpected arguments.");
            eprintln!("Run `taida ingot update --help` for usage.");
            std::process::exit(1);
        }
    }

    // Find project root by looking for packages.tdm
    let project_dir = find_packages_tdm().unwrap_or_else(|| {
        eprintln!("No packages.tdm found in current directory or parent directories.");
        eprintln!("Run 'taida init' to create a new project.");
        std::process::exit(1);
    });

    // Parse manifest
    let manifest = match pkg::manifest::Manifest::from_dir(&project_dir) {
        Ok(Some(m)) => m,
        Ok(None) => {
            eprintln!("No packages.tdm found in '{}'", project_dir.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error parsing packages.tdm: {}", e);
            std::process::exit(1);
        }
    };

    if manifest.deps.is_empty() {
        println!("No dependencies defined in packages.tdm");
        return;
    }

    println!("Updating dependencies for '{}'...", manifest.name);

    // Resolve all dependencies with force-remote (bypass local cache for generations)
    let result = pkg::resolver::resolve_deps_update(&manifest);

    // Report errors
    for err in &result.errors {
        eprintln!("  ERROR: {}", err);
    }

    // Install resolved dependencies (recreate symlinks)
    let mut addon_map: std::collections::BTreeMap<String, pkg::lockfile::LockedAddon> =
        std::collections::BTreeMap::new();
    if !result.resolved.is_empty() {
        match pkg::resolver::install_deps(&manifest, &result) {
            Ok(()) => {
                for pkg in &result.packages {
                    let source_label = match &pkg.source {
                        pkg::provider::PackageSource::Path(p) => format!("path:{}", p),
                        pkg::provider::PackageSource::CoreBundled => "bundled".to_string(),
                        pkg::provider::PackageSource::Store { org, name } => {
                            format!("github:{}/{}", org, name)
                        }
                    };
                    println!("  {} @{} ({})", pkg.name, pkg.version, source_label);
                }
                println!(
                    "Updated {} package(s) in .taida/deps/",
                    result.packages.len()
                );
            }
            Err(e) => {
                eprintln!("Error installing dependencies: {}", e);
                std::process::exit(1);
            }
        }

        // Install addon prebuilds after deps are recreated.
        // Without this, `taida ingot update` destroys addon binaries because
        // `install_deps` recreates `.taida/deps` from scratch.
        let lock_path = project_dir.join(".taida").join("taida.lock");
        let existing_lock = pkg::lockfile::Lockfile::read(&lock_path).unwrap_or(None);
        addon_map = match pkg::resolver::install_addon_prebuilds(
            &manifest,
            &result,
            false, // force_refresh: update fetches latest versions but does not bypass cache
            existing_lock.as_ref(),
            allow_local_addon_build,
            pkg::resolver::AddonInstallPolicy::from_manifest(&manifest, false),
        ) {
            Ok(map) => map,
            Err(e) => {
                eprintln!("Error installing addon prebuilds: {}", e);
                std::process::exit(1);
            }
        };

        if !addon_map.is_empty() {
            for (pkg_name, addon) in &addon_map {
                println!("  Addon {} @ {} ({})", pkg_name, addon.target, addon.sha256);
            }
        }
    }

    // Preserve addon stanzas when writing the lockfile.
    // The old write_lockfile call would discard all [[package.addon]] entries.
    match pkg::resolver::write_lockfile_with_addons(&manifest, &result, &addon_map) {
        Ok(()) => println!("Updated taida.lock"),
        Err(e) => eprintln!("Warning: could not write lockfile: {}", e),
    }

    if !result.errors.is_empty() {
        eprintln!("\nSome dependencies could not be resolved. See errors above.");
        std::process::exit(1);
    }
}

// ── Publish subcommand ─────────────────────────────────

#[cfg(feature = "community")]
/// `taida ingot publish` is now a tag-push-only command.
///
/// Flow:
///
/// 1. Find the `packages.tdm` in the current tree and parse it.
/// 2. Validate the manifest identity (`<<<@version owner/name`
/// required; bare names are rejected).
/// 3. Cross-check identity against `origin` (GitHub URL, exact
/// `owner/repo` match).
/// 4. Check the working tree is clean.
/// 5. Compute the next version from the public API diff (or honour
/// `--force-version`).
/// 6. Detect tag collision (reject unless `--retag`).
/// 7. `--dry-run` prints the plan and exits.
/// 8. Otherwise, `git tag <next> && git push origin <tag>`. Done.
///
/// `taida ingot publish` does not build cdylibs, compute SHA-256, mutate
/// `packages.tdm`, push to `main`, or call `gh release create`. All
/// release artefact work is delegated to the addon
/// `release.yml` running as `github-actions[bot]`.
fn run_publish(args: &[String]) {
    // ── CLI parsing ──────────────────────────────────────
    let mut label: Option<String> = None;
    let mut force_version: Option<String> = None;
    let mut retag = false;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_publish_help();
                return;
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --label.");
                    eprintln!("Run `taida ingot publish --help` for usage.");
                    std::process::exit(1);
                }
                label = Some(args[i].clone());
            }
            "--force-version" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Missing value for --force-version.");
                    eprintln!("Run `taida ingot publish --help` for usage.");
                    std::process::exit(1);
                }
                force_version = Some(args[i].clone());
            }
            "--retag" => retag = true,
            "--dry-run" => dry_run = true,
            raw if raw.starts_with("--dry-run=") => {
                eprintln!(
                    "`--dry-run=<mode>` was removed in @c.14.rc1. Use plain `--dry-run` instead."
                );
                eprintln!("Run `taida ingot publish --help` for the new flow.");
                std::process::exit(1);
            }
            "--target" => {
                eprintln!(
                    "`--target` was removed in @c.14.rc1. `taida ingot publish` now only pushes a git tag; \
                     addon builds happen in CI via `.github/workflows/release.yml`."
                );
                eprintln!("Run `taida ingot publish --help` for the new flow.");
                std::process::exit(1);
            }
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for publish: {}", raw);
                eprintln!("Run `taida ingot publish --help` for usage.");
                std::process::exit(1);
            }
            other => {
                eprintln!("Unexpected argument for publish: {}", other);
                eprintln!("Run `taida ingot publish --help` for usage.");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // ── Project discovery ──────────────────────────────────
    let project_dir = find_packages_tdm().unwrap_or_else(|| {
        eprintln!("No packages.tdm found in current directory or parent directories.");
        eprintln!("Run 'taida init' to create a new project.");
        std::process::exit(1);
    });

    let manifest = match pkg::manifest::Manifest::from_dir(&project_dir) {
        Ok(Some(m)) => m,
        Ok(None) => {
            eprintln!("No packages.tdm found in '{}'", project_dir.display());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error parsing packages.tdm: {}", e);
            std::process::exit(1);
        }
    };

    // ── Invariant: working tree must be clean ──────────────
    if let Err(e) = pkg::publish::check_worktree_clean(&project_dir) {
        eprintln!("Publish refused: {}", e);
        std::process::exit(1);
    }

    // ── Plan ───────────────────────────────────────────────
    let plan = match pkg::publish::plan_publish(
        &project_dir,
        &manifest,
        label.as_deref(),
        force_version.as_deref(),
        retag,
    ) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("Publish refused: {}", e);
            std::process::exit(1);
        }
    };

    // ── Dry-run exits after printing the plan ──────────────
    if dry_run {
        print!("{}", pkg::publish::render_plan(&plan));
        return;
    }

    // ── Authentication check (real run only) ───────────────
    if let Err(e) = pkg::publish::check_gh_auth() {
        eprintln!("Publish refused: {}", e);
        std::process::exit(1);
    }

    // ── Tag + push (the only git-mutating step) ────────────
    if let Err(e) = pkg::publish::tag_and_push(&project_dir, &plan.next_version, plan.retag) {
        eprintln!("Publish failed: {}", e);
        std::process::exit(1);
    }

    // ── Report and exit ────────────────────────────────────
    println!(
        "Pushed tag {} for {} to {}.",
        plan.next_version, plan.package_id, plan.remote
    );
    println!("CI (`.github/workflows/release.yml`) will build artefacts and create the release.");
}

fn run_cache(args: &[String]) {
    if args.is_empty() || args.iter().any(|a| is_help_flag(a.as_str())) {
        println!("Usage: taida ingot cache <command> [options]");
        println!();
        println!("Commands:");
        println!("  clean                       Remove cached WASM runtime .o files (default)");
        println!("  clean --addons              Remove cached addon prebuild binaries");
        println!("                              (prunes ~/.taida/addon-cache/)");
        println!("  clean --store [--yes]       Prune ~/.taida/store/ (shows a summary");
        println!("                              first; then asks to confirm interactively on a");
        println!("                              TTY, or requires --yes in non-TTY contexts)");
        println!("  clean --store-pkg <org>/<name>   Prune a single store package");
        println!("                              (no confirmation prompt; scope is narrow)");
        println!("  clean --all [--yes]         Remove WASM + addon cache + store");
        return;
    }

    match args[0].as_str() {
        "clean" => {
            // RC15B-001: parse optional --addons / --all flags.
            // C17-3: add --store / --store-pkg / --yes flags.
            let mut clean_wasm = true;
            let mut clean_addons = false;
            let mut clean_store = false;
            let mut store_pkg: Option<String> = None;
            let mut assume_yes = false;

            let mut i = 1;
            while i < args.len() {
                let extra = args[i].as_str();
                match extra {
                    "--addons" => {
                        clean_wasm = false;
                        clean_addons = true;
                    }
                    "--store" => {
                        clean_wasm = false;
                        clean_store = true;
                    }
                    "--store-pkg" => {
                        clean_wasm = false;
                        i += 1;
                        if i >= args.len() {
                            eprintln!("Missing value for --store-pkg. Expected <org>/<name>.");
                            std::process::exit(1);
                        }
                        store_pkg = Some(args[i].clone());
                    }
                    "--all" => {
                        clean_wasm = true;
                        clean_addons = true;
                        clean_store = true;
                    }
                    "--yes" | "-y" => {
                        assume_yes = true;
                    }
                    other => {
                        eprintln!(
                            "Unknown flag '{}' for 'taida ingot cache clean'. \
                             Use --addons, --store, --store-pkg <org>/<name>, --all, or no flag.",
                            other
                        );
                        std::process::exit(1);
                    }
                }
                i += 1;
            }

            // --store-pkg is mutually exclusive with --store / --all:
            // targeted prune should not also wipe the whole store.
            if store_pkg.is_some() && (clean_store || (clean_wasm && clean_addons)) {
                eprintln!(
                    "--store-pkg cannot be combined with --store or --all. \
                     Use one or the other."
                );
                std::process::exit(1);
            }

            if clean_wasm {
                run_cache_clean();
            }
            if clean_addons {
                run_cache_clean_addons();
            }
            if clean_store {
                run_cache_clean_store(assume_yes);
            }
            if let Some(pkg) = store_pkg {
                run_cache_clean_store_pkg(&pkg);
            }
        }
        other => {
            eprintln!(
                "Unknown cache command '{}'. Use 'taida ingot cache clean'.",
                other
            );
            std::process::exit(1);
        }
    }
}

fn run_cache_clean() {
    // RCB-56: Use absolute CWD to match run_build()'s input_path.parent() behavior.
    // Both now resolve .taida/cache/wasm-rt/ from an absolute path.
    let project_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cache_dir = codegen::driver::default_wasm_cache_dir(Some(&project_dir));

    if !cache_dir.exists() {
        println!("No cache directory found at '{}'.", cache_dir.display());
        return;
    }

    let mut removed = 0usize;
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            // Remove cached .o files and temp files, preserve 'include/' dir
            if (fname.ends_with(".o") || fname.ends_with(".tmp.c") || fname.ends_with(".tmp.o"))
                && fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }

    if removed > 0 {
        println!(
            "Cleaned {} cached file(s) from '{}'.",
            removed,
            cache_dir.display()
        );
    } else {
        println!(
            "Cache directory '{}' is already clean.",
            cache_dir.display()
        );
    }
}

// RC15B-001: prune the addon prebuild cache at ~/.taida/addon-cache/.
//
// The directory tree is walked by `clean_addon_cache`, which preserves
// user-placed files (anything that is not a recognised addon binary or
// `.manifest-sha256` sidecar) so a confused user can still inspect the
// directory after the command runs.
fn run_cache_clean_addons() {
    match taida::addon::prebuild_fetcher::clean_addon_cache() {
        Ok(summary) => {
            if !summary.root_existed {
                println!("No addon cache found at '{}'.", summary.root.display());
                return;
            }
            let total = summary.binaries_removed + summary.sidecars_removed;
            if total == 0 {
                println!(
                    "Addon cache at '{}' is already clean.",
                    summary.root.display()
                );
            } else {
                let mib = summary.bytes_freed as f64 / (1024.0 * 1024.0);
                println!(
                    "Cleaned {} addon binary file(s) and {} sidecar file(s) ({:.2} MiB) from '{}'.",
                    summary.binaries_removed,
                    summary.sidecars_removed,
                    mib,
                    summary.root.display()
                );
            }
        }
        Err(e) => {
            eprintln!("Error cleaning addon cache: {}", e);
            std::process::exit(1);
        }
    }
}

// C17-3: prune `~/.taida/store/` (all packages, all versions).
//
// Shows a summary first. Requires confirmation (`y` / `yes` / `Y` / `YES`
// on stdin) unless `--yes` is passed. Non-TTY stdin must pass `--yes`
// explicitly so scripts do not wipe the store accidentally.
fn run_cache_clean_store(assume_yes: bool) {
    let store_root = match taida::util::taida_home_dir() {
        Ok(home) => home.join(".taida").join("store"),
        Err(e) => {
            eprintln!("Cannot locate taida home directory: {}", e);
            std::process::exit(1);
        }
    };
    let summary = match taida::pkg::store::summarize_store_root(&store_root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error reading store: {}", e);
            std::process::exit(1);
        }
    };
    if !summary.root_existed {
        println!("No store cache found at '{}'.", summary.root.display());
        return;
    }
    if summary.packages_removed == 0 && summary.scratch_removed == 0 {
        println!(
            "Store cache at '{}' is already clean.",
            summary.root.display()
        );
        return;
    }

    // Show summary.
    let mib = summary.bytes_removed as f64 / (1024.0 * 1024.0);
    println!(
        "Store cache at '{}' contains {} package(s), {:.2} MiB.",
        summary.root.display(),
        summary.packages_removed,
        mib
    );
    // C17B-011: report scratch (leftover .tmp-*, .refresh-staging-*)
    // separately so the user sees what is being cleaned up without the
    // count inflating the package number.
    if summary.scratch_removed > 0 {
        println!(
            "  ... and {} leftover scratch directory(ies) from past installs",
            summary.scratch_removed
        );
    }
    // Preview the first few so a user can sanity-check.
    let preview_n = 10usize;
    for name in summary.packages.iter().take(preview_n) {
        println!("  {}", name);
    }
    if summary.packages.len() > preview_n {
        println!("  ... and {} more", summary.packages.len() - preview_n);
    }

    if !assume_yes {
        use std::io::Write;
        let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
        if !is_tty {
            eprintln!(
                "Refusing to prune store in a non-TTY context without --yes. \
                 Re-run with `taida ingot cache clean --store --yes`."
            );
            std::process::exit(1);
        }
        print!("Remove all {} package(s)? [y/N] ", summary.packages_removed);
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            eprintln!("No input received; aborting.");
            std::process::exit(1);
        }
        let answer = answer.trim();
        if !matches!(answer, "y" | "Y" | "yes" | "YES") {
            println!("Aborted.");
            return;
        }
    }

    match taida::pkg::store::prune_store_root(&store_root) {
        Ok(report) => {
            let mib = report.bytes_removed as f64 / (1024.0 * 1024.0);
            println!(
                "Removed {} package(s) ({:.2} MiB) from '{}'.",
                report.packages_removed,
                mib,
                report.root.display()
            );
        }
        Err(e) => {
            eprintln!("Error pruning store: {}", e);
            std::process::exit(1);
        }
    }
}

// C17-3: prune a single package from the store (all versions of
// `<org>/<name>/*`). No confirmation is required since the scope is
// narrow.
fn run_cache_clean_store_pkg(pkg_spec: &str) {
    let (org, name) = match pkg_spec.split_once('/') {
        Some((o, n)) if !o.is_empty() && !n.is_empty() && !n.contains('/') => (o, n),
        _ => {
            eprintln!(
                "Invalid --store-pkg value '{}'. Expected <org>/<name>.",
                pkg_spec
            );
            std::process::exit(1);
        }
    };
    let store_root = match taida::util::taida_home_dir() {
        Ok(home) => home.join(".taida").join("store"),
        Err(e) => {
            eprintln!("Cannot locate taida home directory: {}", e);
            std::process::exit(1);
        }
    };
    match taida::pkg::store::prune_store_package(&store_root, org, name) {
        Ok(report) => {
            if !report.root_existed {
                println!("No store cache found at '{}'.", report.root.display());
                return;
            }
            if report.packages_removed == 0 {
                println!(
                    "Package '{}/{}' not found in store at '{}'.",
                    org,
                    name,
                    report.root.display()
                );
                return;
            }
            let mib = report.bytes_removed as f64 / (1024.0 * 1024.0);
            println!(
                "Removed {} version(s) of {}/{} ({:.2} MiB) from '{}'.",
                report.packages_removed,
                org,
                name,
                mib,
                report.root.display()
            );
        }
        Err(e) => {
            eprintln!("Error pruning store package: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_doc(args: &[String]) {
    if args.iter().any(|a| is_help_flag(a.as_str())) {
        print_doc_help();
        return;
    }

    if args.is_empty() || args[0] != "generate" {
        eprintln!("Unknown or missing subcommand for doc.");
        eprintln!("Run `taida doc --help` for usage.");
        std::process::exit(1);
    }

    // Parse args after "generate"
    let gen_args = &args[1..];
    let mut input_path: Option<String> = None;
    let mut output_path: Option<String> = None;

    let mut i = 0;
    while i < gen_args.len() {
        match gen_args[i].as_str() {
            "--help" | "-h" => {
                print_doc_help();
                return;
            }
            "-o" | "--output" => {
                i += 1;
                if i >= gen_args.len() {
                    eprintln!("Missing value for -o/--output.");
                    eprintln!("Run `taida doc --help` for usage.");
                    std::process::exit(1);
                }
                output_path = Some(gen_args[i].clone());
            }
            raw if raw.starts_with('-') => {
                eprintln!("Unknown option for doc generate: {}", raw);
                eprintln!("Run `taida doc --help` for usage.");
                std::process::exit(1);
            }
            _ => {
                if input_path.is_some() {
                    eprintln!("Only one <PATH> is accepted for taida doc generate.");
                    std::process::exit(1);
                }
                input_path = Some(gen_args[i].clone());
            }
        }
        i += 1;
    }

    let input = match input_path {
        Some(p) => p,
        None => {
            eprintln!("Missing <PATH> argument.");
            eprintln!("Run `taida doc --help` for usage.");
            std::process::exit(1);
        }
    };

    let target_path = Path::new(&input);

    // Collect .td files
    let td_files: Vec<PathBuf> = if target_path.is_dir() {
        let files = collect_td_files(target_path);
        if files.is_empty() {
            eprintln!("No .td files found in '{}'", input);
            std::process::exit(1);
        }
        files
    } else {
        vec![target_path.to_path_buf()]
    };

    let mut all_output = String::new();

    for td_file in &td_files {
        let source = match fs::read_to_string(td_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading file '{}': {}", td_file.display(), e);
                continue;
            }
        };

        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            for err in &parse_errors {
                eprintln!("{}: {}", td_file.display(), err);
            }
            continue;
        }

        let module_name = td_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let module_doc = doc::extract_docs(&program, module_name);
        let markdown = doc::render_markdown(&module_doc);

        if !markdown.trim().is_empty() {
            all_output.push_str(&markdown);
        }
    }

    match output_path {
        Some(out) => {
            // Create parent directory if needed
            if let Some(parent) = Path::new(&out).parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::write(&out, &all_output) {
                Ok(_) => println!("Documentation generated: {}", out),
                Err(e) => {
                    eprintln!("Error writing '{}': {}", out, e);
                    std::process::exit(1);
                }
            }
        }
        None => {
            print!("{}", all_output);
        }
    }
}

fn find_packages_tdm_from(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        if dir.join("packages.tdm").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn find_packages_tdm() -> Option<PathBuf> {
    let dir = env::current_dir().ok()?;
    find_packages_tdm_from(&dir)
}

// ── LSP server ─────────────────────────────────────────

#[cfg(feature = "lsp")]
fn run_lsp(args: &[String]) {
    match args {
        [] => {}
        [arg] if is_help_flag(arg.as_str()) => {
            print_lsp_help();
            return;
        }
        _ => {
            eprintln!("Unexpected arguments.");
            eprintln!("Run `taida lsp --help` for usage.");
            std::process::exit(1);
        }
    }

    // N-54: Tokio runtime creation fails only under severe resource
    // exhaustion (e.g. file descriptor limit reached). In such cases
    // there is no meaningful recovery, so panic with a clear message.
    let rt = tokio::runtime::Runtime::new()
        .expect("failed to create Tokio runtime for LSP server (possible fd/resource exhaustion)");
    rt.block_on(taida::lsp::server::run_server());
}

// ── REPL ────────────────────────────────────────────────

fn repl(no_check: bool) {
    let mut interpreter = Interpreter::new();

    loop {
        print!("taida> ");
        // N-45: REPL stdout flush — failure means the output pipe is broken
        // (e.g. piped into a closed process), in which case continuing the
        // REPL loop is pointless. Use `ok()` to silently exit on next read.
        if io::stdout().flush().is_err() {
            break;
        }

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => {
                // EOF
                println!();
                break;
            }
            Ok(_) => {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }

                let (program, parse_errors) = parse(input);
                if !parse_errors.is_empty() {
                    for err in &parse_errors {
                        eprintln!("  {}", err);
                    }
                    continue;
                }

                // Type checking in REPL (warn but don't abort)
                if !no_check {
                    let mut checker = TypeChecker::new();
                    checker.set_compile_target(CompileTarget::Interpreter);
                    checker.check_program(&program);
                    if !checker.errors.is_empty() {
                        for err in &checker.errors {
                            eprintln!("  {}", err);
                        }
                        // Continue execution despite type errors in REPL
                    }
                }

                match interpreter.eval_program(&program) {
                    Ok(val) => {
                        for line in &interpreter.output {
                            println!("{}", line);
                        }
                        interpreter.output.clear();
                        if !matches!(val, taida::interpreter::Value::Unit) {
                            println!("  {}", val);
                        }
                    }
                    Err(e) => {
                        for line in &interpreter.output {
                            println!("{}", line);
                        }
                        interpreter.output.clear();
                        eprintln!("  {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::build_descriptor::{
        BuildUnitDescriptor, target_incompatible_import, validate_target_closure_modules,
    };
    use std::time::{SystemTime, UNIX_EPOCH};
    use taida::parser::ImportStmt;

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("taida-{}-{}-{}", name, std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn js_build_helper_emits_program_body_for_stdin_source() {
        let dir = temp_test_dir("stdin-js-build");
        let out = dir.join("stdin.js");
        let mut stats = CompileDiagStats::default();

        transpile_js_source_to_output(
            "opt <= Lax[42]()\nstdout(opt.hasValue().toString())\n",
            "/dev/stdin",
            None,
            &out,
            None,
            false,
            DiagFormat::Text,
            &mut stats,
            None,
            None,
            None,
        );

        let js = fs::read_to_string(&out).unwrap();
        assert!(js.contains("const opt = __taida_solidify(Lax(42));"));
        // C12-2b: `.toString()` is routed through `__taida_to_string` so
        // plain BuchiPacks render as `@(...)` instead of the JS default
        // `[object Object]`. The receiver is still wrapped — here the
        // hasValue() call returns a primitive Boolean, which the helper
        // formats via `String(v)` (matches interpreter / native).
        assert!(js.contains("__taida_stdout(__taida_to_string(opt.hasValue()));"));

        fs::remove_file(&out).unwrap();
        fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn cli_version_matches_embedded_build_metadata() {
        // taida_version() is the single source of truth — verify it returns
        // a non-empty string (exact value depends on build environment).
        let version = taida_version();
        assert!(!version.is_empty(), "taida_version() should not be empty");
    }

    /// `validate_target_closure_modules` rejects any closure module that
    /// has parse errors with `[E1941]` so a TOCTOU race window between
    /// `module_graph::collect_local_modules` and the inner re-read cannot
    /// silently downgrade a target-incompatibility diagnostic. Exercised
    /// directly here because the upstream `collect_local_modules` step in
    /// `validate_target_closure` would otherwise reject the same fixture
    /// before the inner loop runs, leaving the inner hard-fail untested
    /// in end-to-end flows.
    #[test]
    fn validate_target_closure_modules_rejects_parse_error_inner() {
        let dir = temp_test_dir("validate-closure-inner-parse");
        let entry = dir.join("entry.td");
        fs::write(&entry, "stdout(\"entry\")\n").expect("write entry");
        let bad = dir.join("bad.td");
        fs::write(&bad, "let bad = (\n").expect("write bad module");

        let unit = BuildUnitDescriptor {
            symbol: "frontendA".to_string(),
            name: "frontend-a".to_string(),
            target: BuildTarget::WasmMin,
            entry_symbol: "entryMain".to_string(),
            entry_path: Some(entry.clone()),
            handler: None,
            route_assets: Vec::new(),
            before_hooks: Vec::new(),
        };

        let err = validate_target_closure_modules(&unit, &entry, std::slice::from_ref(&bad))
            .expect_err(
                "TOCTOU defence must reject any closure module that fails to parse on re-read",
            );
        assert_eq!(err.code, "E1941");
        assert!(
            err.message.contains("frontend-a") && err.message.contains("bad.td"),
            "diagnostic must mention the unit and offending module: {}",
            err.message
        );
        assert!(
            err.message.to_ascii_lowercase().contains("parse error"),
            "diagnostic must surface the parse error context: {}",
            err.message
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// Sibling guarantee: when the closure target is not restricted (e.g.
    /// `js`), the inner re-parse path must short-circuit so that benign
    /// build pipelines that lower through unrestricted targets do not pay
    /// the wasm-only TOCTOU cost.
    #[test]
    fn validate_target_closure_modules_skips_inner_parse_for_unrestricted_target() {
        let dir = temp_test_dir("validate-closure-inner-skip");
        let entry = dir.join("entry.td");
        fs::write(&entry, "stdout(\"entry\")\n").expect("write entry");
        let bad = dir.join("bad.td");
        fs::write(&bad, "let bad = (\n").expect("write bad module");

        let unit = BuildUnitDescriptor {
            symbol: "serverA".to_string(),
            name: "server-a".to_string(),
            target: BuildTarget::Js,
            entry_symbol: "entryMain".to_string(),
            entry_path: Some(entry.clone()),
            handler: None,
            route_assets: Vec::new(),
            before_hooks: Vec::new(),
        };

        validate_target_closure_modules(&unit, &entry, &[bad])
            .expect("non-wasm targets must skip the closure re-parse pass");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrangler_manifest_reader_maps_cloudflare_bindings() {
        let source = r#"
{
  // JSONC comments and trailing commas are accepted.
  "name": "edge-app",
  "route": "https://example.com/*",
  "d1_databases": [{ "binding": "DB" }],
  "kv_namespaces": [{ "binding": "CACHE" }],
  "durable_objects": {
    "bindings": [{ "name": "COUNTER", "class_name": "Counter" }],
  },
  "r2_buckets": [{ "binding": "ASSETS" }],
  "queues": {
    "producers": [{ "binding": "OUTBOX", "queue": "outbox" }],
  },
  "services": [{ "binding": "API", "service": "api" }],
}
"#;

        let capabilities =
            parse_wrangler_host_capability_manifest_str(source).expect("manifest should parse");
        assert_eq!(
            capabilities,
            vec![
                ("DB".to_string(), "cloudflare/d1".to_string()),
                ("CACHE".to_string(), "cloudflare/kv".to_string()),
                ("COUNTER".to_string(), "cloudflare/do_namespace".to_string()),
                ("ASSETS".to_string(), "cloudflare/r2".to_string()),
                (
                    "OUTBOX".to_string(),
                    "cloudflare/queue_producer".to_string()
                ),
                ("API".to_string(), "cloudflare/fetcher".to_string()),
            ]
        );
    }

    #[test]
    fn wrangler_manifest_reader_stops_at_project_marker() {
        let outer = temp_test_dir("wrangler-outer");
        let project = outer.join("project");
        let src = project.join("src");
        fs::create_dir_all(&src).expect("create project tree");
        fs::write(outer.join("wrangler.jsonc"), r#"{ "d1_databases": [] }"#)
            .expect("write outer wrangler");
        fs::write(project.join("taida.toml"), "").expect("write project marker");
        let td = src.join("main.td");
        fs::write(&td, "stdout(\"ok\")\n").expect("write source");

        assert!(
            find_wrangler_manifest_for_source(&td).is_none(),
            "manifest search must not cross the project marker"
        );

        fs::remove_dir_all(&outer).ok();
    }

    fn parse_single_import(source: &str) -> ImportStmt {
        let (program, errors) = parse(source);
        assert!(errors.is_empty(), "fixture parse errors: {errors:?}");
        program
            .statements
            .into_iter()
            .find_map(|stmt| match stmt {
                Statement::Import(import) => Some(import),
                _ => None,
            })
            .expect("fixture must contain an import")
    }

    #[test]
    fn wasm_descriptor_closure_matrix_rejects_incompatible_core_imports() {
        let net = parse_single_import(">>> taida-lang/net@a.1 => @(httpServe)\n");
        let terminal = parse_single_import(">>> taida-lang/terminal@a.1 => @(readKey)\n");
        let os_env = parse_single_import(">>> taida-lang/os@a.1 => @(EnvVar, allEnv)\n");
        let os_file = parse_single_import(">>> taida-lang/os@a.1 => @(Read)\n");
        let os_process = parse_single_import(">>> taida-lang/os@a.1 => @(run)\n");

        assert_eq!(
            target_incompatible_import(BuildTarget::WasmMin, &os_env).as_deref(),
            Some("taida-lang/os")
        );
        assert_eq!(
            target_incompatible_import(BuildTarget::WasmWasi, &net).as_deref(),
            Some("taida-lang/net")
        );
        assert_eq!(
            target_incompatible_import(BuildTarget::WasmFull, &net).as_deref(),
            Some("taida-lang/net")
        );
        assert_eq!(
            target_incompatible_import(BuildTarget::WasmEdge, &terminal).as_deref(),
            Some("taida-lang/terminal")
        );
        assert!(
            target_incompatible_import(BuildTarget::WasmEdge, &os_env).is_none(),
            "wasm-edge supports environment-only OS imports"
        );
        assert_eq!(
            target_incompatible_import(BuildTarget::WasmEdge, &os_file).as_deref(),
            Some("taida-lang/os::Read")
        );
        assert!(
            target_incompatible_import(BuildTarget::WasmWasi, &os_file).is_none(),
            "wasm-wasi supports the WASI file subset"
        );
        assert_eq!(
            target_incompatible_import(BuildTarget::WasmFull, &os_process).as_deref(),
            Some("taida-lang/os::run")
        );
    }
}
