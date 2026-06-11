//! commands — split out of src/main.rs (pure move).
//! Behaviour unchanged; imports added per cargo check.

use serde_json::json;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use taida::doc;
use taida::graph::ai_format;
use taida::graph::verify;
use taida::interpreter::Interpreter;
use taida::parser::parse;
use taida::pkg;
use taida::types::{CompileTarget, TypeChecker};
use taida::version::taida_version;

use crate::cli::help::{
    print_doc_help, print_graph_help, print_graph_summary_help, print_init_help, print_lsp_help,
    print_upgrade_help,
};
use crate::cli::ingot::find_packages_tdm;
use crate::cli::way::collect_td_files;
use crate::{is_help_flag, reject_removed_migration_command};

pub(crate) fn run_source(source: &str, filename: &str, no_check: bool) {
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
    //
    // F62B-027: evaluation runs on a dedicated thread with a large stack.
    // Each Taida call costs multiple Rust frames, so the raised
    // MAX_CALL_DEPTH (8192, matching the failure depth class of the
    // compiled backends) does not fit in the default 8 MiB main stack.
    // The reservation is virtual address space — pages are committed only
    // as actually used.
    let filename_owned = filename.to_string();
    let eval_thread = std::thread::Builder::new()
        .name("taida-eval".to_string())
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run_program_on_eval_thread(program, &filename_owned))
        .expect("spawn taida eval thread");
    match eval_thread.join() {
        Ok(()) => {}
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn run_program_on_eval_thread(program: taida::parser::Program, filename: &str) {
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
            // F62B-032: the gorilla literal is the documented fixed
            // `exit(1)` immediate termination — the interpreter used to
            // fall through here, display the gorilla value, and exit 0
            // (native already exits 1).
            if matches!(val, taida::interpreter::Value::Gorilla) {
                std::process::exit(1);
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

#[cfg(feature = "community")]
pub(crate) fn run_upgrade(args: &[String]) {
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

pub(crate) fn run_graph(args: &[String]) {
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

pub(crate) fn run_graph_summary(args: &[String]) {
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

pub(crate) fn format_graph_summary_sarif(summary_json: &str) -> String {
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

pub(crate) fn run_init(args: &[String]) {
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

pub(crate) fn run_doc(args: &[String]) {
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

#[cfg(feature = "lsp")]
pub(crate) fn run_lsp(args: &[String]) {
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

pub(crate) fn repl(no_check: bool) {
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
