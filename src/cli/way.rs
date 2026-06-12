//! way — split out of src/main.rs (pure move).
//! Behaviour unchanged; imports added per cargo check.

use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

use taida::diagnostics::split_diag_code_and_hint;
use taida::graph::verify;
use taida::parser::{BuchiField, Expr, FieldDef, FuncDef, Program, Statement, parse};
use taida::types::{CompileTarget, TypeChecker};

use crate::cli::build::{CompileDiagStats, DiagFormat, emit_compile_diag_jsonl};
use crate::cli::help::{
    print_check_help, print_lint_help, print_todo_help, print_verify_help, print_way_help,
};
use crate::{is_help_flag, reject_removed_migration_command};

pub(crate) fn reject_no_check_under_way() -> ! {
    eprintln!(
        "--no-check is not allowed under 'taida way'. The way hub exists to run quality checks."
    );
    std::process::exit(2);
}

pub(crate) fn way_should_fail(errors: usize, warnings: usize, strict: bool) -> bool {
    errors > 0 || (strict && warnings > 0)
}

pub(crate) fn parse_way_format_or_exit(raw: &str, command: &str) -> WayFormat {
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

pub(crate) fn push_way_options_args(out: &mut Vec<String>, options: WayOptions) {
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

pub(crate) fn run_way(args: &[String], no_check: bool) {
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

pub(crate) fn run_way_full(args: &[String]) {
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

pub(crate) fn run_check_cmd(args: &[String]) {
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

pub(crate) fn emit_check_diagnostics(
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

pub(crate) fn check_diagnostics_json(
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

pub(crate) fn run_lint_cmd(args: &[String]) {
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

pub(crate) fn collect_release_gate_sites_for_files(td_files: &[PathBuf]) -> Vec<TodoStubSite> {
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

pub(crate) fn collect_td_files(dir: &Path) -> Vec<std::path::PathBuf> {
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

pub(crate) fn extract_string_field(fields: &[BuchiField], name: &str) -> Option<String> {
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

pub(crate) fn scan_field_defaults(field: &FieldDef, file: &str, out: &mut TodoScanResult) {
    if let Some(default_expr) = &field.default_value {
        scan_expr_for_todo(default_expr, file, out);
    }
    if let Some(method) = &field.method_def {
        scan_func_for_todo(method, file, out);
    }
}

pub(crate) fn scan_func_for_todo(func: &FuncDef, file: &str, out: &mut TodoScanResult) {
    for param in &func.params {
        if let Some(default_expr) = &param.default_value {
            scan_expr_for_todo(default_expr, file, out);
        }
    }
    for stmt in &func.body {
        scan_stmt_for_todo(stmt, file, out);
    }
}

pub(crate) fn scan_stmt_for_todo(stmt: &Statement, file: &str, out: &mut TodoScanResult) {
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

pub(crate) fn scan_expr_for_todo(expr: &Expr, file: &str, out: &mut TodoScanResult) {
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
        Expr::Block(stmts, _) => {
            for stmt in stmts {
                scan_stmt_for_todo(stmt, file, out);
            }
        }
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

pub(crate) fn scan_program_for_todo(program: &Program, file: &Path) -> TodoScanResult {
    let file_label = file.to_string_lossy().to_string();
    let mut out = TodoScanResult::default();
    for stmt in &program.statements {
        scan_stmt_for_todo(stmt, &file_label, &mut out);
    }
    out
}

pub(crate) fn resolve_local_import_path(base_dir: &Path, import_path: &str) -> PathBuf {
    base_dir.join(import_path)
}

pub(crate) fn collect_release_scan_files(target_path: &Path) -> Vec<PathBuf> {
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

pub(crate) fn scan_release_gate_sites(target_path: &Path) -> Vec<TodoStubSite> {
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

pub(crate) fn report_release_gate_violations(
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

pub(crate) fn run_todo(args: &[String]) {
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

pub(crate) fn run_verify(args: &[String]) {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WayFormat {
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
pub(crate) struct WayOptions {
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

#[derive(Clone, Debug)]
pub(crate) struct CheckDiagnostic {
    stage: &'static str,
    severity: &'static str,
    code: Option<String>,
    message: String,
    file: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    suggestion: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TodoItem {
    id: Option<String>,
    task: Option<String>,
    file: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct TodoStubSite {
    kind: &'static str,
    file: String,
    line: usize,
    column: usize,
}

#[derive(Default)]
pub(crate) struct TodoScanResult {
    todos: Vec<TodoItem>,
    sites: Vec<TodoStubSite>,
}
