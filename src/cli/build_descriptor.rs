//! build_descriptor — split out of src/main.rs (pure move).
//! Behaviour unchanged; imports added per cargo check.

use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use taida::diagnostics::split_diag_code_and_hint;
use taida::module_graph;
use taida::parser::{BuchiField, Expr, ImportStmt, Program, Statement, parse};

use crate::cli::build::{
    BuildDiagContext, BuildTarget, CompileDiagStats, DiagFormat,
    emit_build_cli_diagnostic_and_exit, emit_compile_diag_jsonl,
};

/// Environment variables forwarded into hook subprocesses after `env_clear()`.
/// Hooks need `PATH` to resolve the command itself, and `HOME` / `LANG` /
/// `LC_ALL` because tooling such as `npm` and `git` refuse to run without them.
/// Anything else must be declared in `BuildHook.env` so descriptor builds stay
/// reproducible across machines.
const HOOK_FORWARDED_ENV_VARS: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL"];

pub(crate) fn build_descriptor_entry_path(input_path: &Path) -> Result<PathBuf, String> {
    if input_path.is_dir() {
        let candidate = input_path.join("main.td");
        if !candidate.exists() || !candidate.is_file() {
            return Err(format!(
                "Build descriptor input not found: {}",
                candidate.display()
            ));
        }
        return Ok(candidate);
    }

    if !input_path.exists() || !input_path.is_file() {
        return Err(format!("Build input not found: {}", input_path.display()));
    }

    Ok(input_path.to_path_buf())
}

pub(crate) fn descriptor_name_from_fields(fields: &[BuchiField], fallback: &str) -> String {
    fields
        .iter()
        .find_map(|field| match (&field.name, &field.value) {
            (name, Expr::StringLit(value, _)) if name == "name" => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn format_descriptor_candidates(candidates: &[String]) -> String {
    if candidates.is_empty() {
        "<none>".to_string()
    } else {
        candidates.join(", ")
    }
}

pub(crate) fn emit_descriptor_build_error_and_exit(
    error: DescriptorBuildError,
    diag_format: DiagFormat,
    compile_stats: &mut CompileDiagStats,
) -> ! {
    compile_stats.errors += 1;
    if diag_format == DiagFormat::Jsonl {
        let build = json!({
            "unit": error.context.unit,
            "target": error.context.target,
            "edge_kind": error.context.edge_kind,
            "dependency_path": if error.context.dependency_path.is_empty() {
                serde_json::Value::Null
            } else {
                json!(error.context.dependency_path)
            },
            "transaction_id": error.context.transaction_id,
            "hook_name": error.context.hook_name,
            "cwd": error.context.cwd,
            "exit_code": error.context.exit_code,
        });
        let rec = json!({
            "schema": "taida.diagnostic.v1",
            "stream": "compile",
            "kind": "error",
            "code": error.code,
            "message": error.message,
            "location": null,
            "suggestion": error.suggestion,
            "stage": "build",
            "severity": "ERROR",
            "build": build,
        });
        println!("{}", rec);
    } else {
        eprintln!("[{}] {}", error.code, error.message);
        if error.context.unit.is_some() || error.context.target.is_some() {
            eprintln!(
                "        unit={} target={}",
                error.context.unit.as_deref().unwrap_or("-"),
                error.context.target.as_deref().unwrap_or("-")
            );
        }
        if let Some(edge) = error.context.edge_kind {
            let path = if error.context.dependency_path.is_empty() {
                "-".to_string()
            } else {
                error.context.dependency_path.join(" -> ")
            };
            eprintln!("        edge={} dependency={}", edge, path);
        }
        if let Some(hook) = error.context.hook_name.as_deref() {
            eprintln!(
                "        hook={} cwd={} exit_code={}",
                hook,
                error.context.cwd.as_deref().unwrap_or("-"),
                error
                    .context
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
        }
        if let Some(suggestion) = error.suggestion {
            eprintln!("        {}", suggestion);
        }
    }
    std::process::exit(error.exit_code);
}

pub(crate) fn descriptor_lock_pid(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("pid")
        .and_then(serde_json::Value::as_u64)
}

pub(crate) fn append_descriptor_cleanup_log(
    build_root: &Path,
    line: &str,
) -> Result<(), DescriptorBuildError> {
    let log_path = build_root.join(".cleanup.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| {
            DescriptorBuildError::new(
                "E1922",
                format!(
                    "Cannot open descriptor staging cleanup log '{}': {}",
                    log_path.display(),
                    e
                ),
            )
        })?;
    writeln!(file, "{}", line).map_err(|e| {
        DescriptorBuildError::new(
            "E1922",
            format!(
                "Cannot write descriptor staging cleanup log '{}': {}",
                log_path.display(),
                e
            ),
        )
    })
}

/// Returns `Some(true)` if `pid` names a live process, `Some(false)` if it
/// is known to be dead, and `None` if the platform has no supported probe.
/// Callers fall back to TTL-only cleanup (and a `pid_alive_check=unsupported`
/// log entry) when this returns `None`, so a missing probe never deletes a
/// staging dir whose owner is still active.
pub(crate) fn descriptor_pid_alive(pid: u64) -> Option<bool> {
    if pid == std::process::id() as u64 {
        return Some(true);
    }

    #[cfg(target_os = "linux")]
    {
        Some(Path::new("/proc").join(pid.to_string()).exists())
    }

    // POSIX (macOS / *BSD): `kill(pid, 0)` performs a permission check
    // without delivering a signal. rc == 0 means the target exists and is
    // owned by us, EPERM means it exists but is owned by another user
    // (still alive), ESRCH means no such pid. Other errnos are treated as
    // "not alive" rather than "alive" to fail closed for cleanup.
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let raw = match i32::try_from(pid) {
            Ok(v) => v,
            Err(_) => return Some(false),
        };
        let rc = unsafe { libc::kill(raw, 0) };
        if rc == 0 {
            return Some(true);
        }
        let err = std::io::Error::last_os_error();
        if matches!(err.raw_os_error(), Some(libc::EPERM)) {
            return Some(true);
        }
        return Some(false);
    }

    // Windows / other targets: no in-tree probe yet — caller should fall
    // back to a shorter TTL and a `pid_alive_check=unsupported` log line.
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

pub(crate) fn cleanup_stale_descriptor_staging(
    build_root: &Path,
) -> Result<(), DescriptorBuildError> {
    let entries = fs::read_dir(build_root).map_err(|e| {
        DescriptorBuildError::new(
            "E1922",
            format!(
                "Cannot scan descriptor build root '{}': {}",
                build_root.display(),
                e
            ),
        )
    })?;
    // Probe support is platform-fixed: when the host has no working
    // `descriptor_pid_alive` probe (e.g. Windows today), the 24 h TTL would
    // let crashed-process staging dirs accumulate disk forever. Fall back
    // to a 4 h TTL and stamp the cleanup log with the unsupported reason
    // so operators have a paper trail. 4 h is chosen as the smallest TTL
    // that still leaves long-running CI / release builds (typically under
    // 1 h end-to-end) a comfortable margin without heartbeat support — the
    // current scheme has no `transaction.json` mtime refresh during a
    // build, so the TTL is the only signal that distinguishes an active
    // owner from a crashed one. A heartbeat / mtime refresh is tracked as
    // a separate hardening item.
    //
    // When the process probe is supported (Linux / POSIX), use a 6 h cap
    // instead of 24 h. `transaction.json` is plain JSON, so anyone who can
    // write to a staging directory could pin its `pid` field to any live
    // process and `touch` the file to keep `mtime` fresh. Capping at 6 h
    // bounds the window during which a spoofed PID can sit on disk.
    let probe_supported = descriptor_pid_alive(std::process::id() as u64).is_some();
    let ttl = if probe_supported {
        std::time::Duration::from_secs(6 * 60 * 60)
    } else {
        std::time::Duration::from_secs(4 * 60 * 60)
    };
    if !probe_supported {
        let _ =
            append_descriptor_cleanup_log(build_root, "scan pid_alive_check=unsupported ttl=4h");
    }
    // Capture the current effective UID once per scan so we can refuse to
    // trust `transaction.json` written by a different owner. On shared
    // multi-user projects another user can otherwise spoof a live PID into
    // our staging directory and survive both the alive probe and mtime TTL.
    let current_uid: Option<u32> = {
        #[cfg(unix)]
        {
            // Safety: `geteuid` is async-signal-safe and returns the
            // calling process's effective UID.
            Some(unsafe { libc::geteuid() } as u32)
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    let now = std::time::SystemTime::now();
    for entry in entries {
        let entry = entry.map_err(|e| {
            DescriptorBuildError::new("E1922", format!("Cannot inspect build root entry: {}", e))
        })?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.starts_with(".tmp-") {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            DescriptorBuildError::new(
                "E1922",
                format!(
                    "Cannot inspect descriptor staging path '{}': {}",
                    path.display(),
                    e
                ),
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }
        // The owner UID check runs before reading `transaction.json`.
        // A co-tenant with write access to the build root could otherwise
        // plant malformed or unreadable JSON and wedge the cleanup pass.
        // Foreign-owned directories are removed without trusting their JSON.
        //
        // TOCTOU note: there is a non-zero gap between `fs::metadata(&path)`
        // here and `fs::remove_dir_all(&path)` below. A racing attacker
        // with same-tree write access could in principle swap the
        // directory in between. We accept the residual risk because:
        //   1. Anyone able to race here already needs write access to
        //      `.taida/build/` — they could just delete the staging
        //      directly anyway.
        //   2. `remove_dir_all` failure on an unexpected layout surfaces
        //      as `[E1922]` rather than being silently swallowed, so
        //      breakage is observable in the cleanup log.
        // A fully race-free implementation would require `openat` +
        // `unlinkat(AT_REMOVEDIR)`, matching the dirfd-based hardening shape
        // used for other cache and staging paths.
        let owner_mismatch = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                match (current_uid, fs::metadata(&path).map(|m| m.uid()).ok()) {
                    (Some(my_uid), Some(staging_uid)) => my_uid != staging_uid,
                    _ => false,
                }
            }
            #[cfg(not(unix))]
            {
                let _ = current_uid;
                false
            }
        };
        if owner_mismatch {
            append_descriptor_cleanup_log(
                build_root,
                &format!(
                    "remove staging={} reason=owner-uid-mismatch pid=-",
                    file_name
                ),
            )?;
            fs::remove_dir_all(&path).map_err(|e| {
                DescriptorBuildError::new(
                    "E1922",
                    format!(
                        "Cannot remove foreign-owned descriptor staging directory '{}': {}",
                        path.display(),
                        e
                    ),
                )
            })?;
            continue;
        }
        let tx_path = path.join("transaction.json");
        if !tx_path.is_file() {
            continue;
        }
        let tx_text = fs::read_to_string(&tx_path).map_err(|e| {
            DescriptorBuildError::new(
                "E1922",
                format!(
                    "Cannot read descriptor transaction file '{}': {}",
                    tx_path.display(),
                    e
                ),
            )
        })?;
        let tx: serde_json::Value = serde_json::from_str(&tx_text).map_err(|e| {
            DescriptorBuildError::new(
                "E1922",
                format!(
                    "Cannot parse descriptor transaction file '{}': {}",
                    tx_path.display(),
                    e
                ),
            )
        })?;
        let pid = tx.get("pid").and_then(serde_json::Value::as_u64);
        let modified = fs::metadata(&tx_path).and_then(|meta| meta.modified()).ok();
        // A forward clock skew (mtime > now) made `duration_since` return Err,
        // which fell through to `unwrap_or(false)` and kept the staging alive
        // even past the TTL. Treat any forward skew as expired so a host
        // with a wandering clock cannot make staging dirs immortal.
        let (expired, clock_skew) = match modified {
            Some(mtime) => match now.duration_since(mtime) {
                Ok(age) => (age > ttl, false),
                Err(_) => (true, true),
            },
            None => (false, false),
        };
        // `descriptor_pid_alive` returns `None` when the host has no probe;
        // treat that as "unknown, not dead" so cleanup waits on TTL alone.
        let dead_pid = pid
            .and_then(descriptor_pid_alive)
            .map(|alive| !alive)
            .unwrap_or(false);
        if !expired && !dead_pid {
            continue;
        }
        let reason = match (expired, dead_pid, clock_skew) {
            (true, true, true) => "clock-skew-and-dead-pid",
            (true, true, false) => "expired-and-dead-pid",
            (true, false, true) => "clock-skew",
            (true, false, false) => "expired",
            (false, true, _) => "dead-pid",
            (false, false, _) => unreachable!("filtered above"),
        };
        append_descriptor_cleanup_log(
            build_root,
            &format!(
                "remove staging={} reason={} pid={}",
                file_name,
                reason,
                pid.map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ),
        )?;
        fs::remove_dir_all(&path).map_err(|e| {
            DescriptorBuildError::new(
                "E1922",
                format!(
                    "Cannot remove stale descriptor staging directory '{}': {}",
                    path.display(),
                    e
                ),
            )
        })?;
    }
    Ok(())
}

pub(crate) fn descriptor_transaction_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after 1970-01-01 (UNIX epoch)")
        .as_nanos();
    format!("{}-{}", std::process::id(), nanos)
}

pub(crate) fn descriptor_project_root(entry_path: &Path) -> Result<PathBuf, DescriptorBuildError> {
    let mut dir = entry_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let start = dir.clone();
    loop {
        if dir.join("packages.tdm").exists()
            || dir.join("taida.toml").exists()
            || dir.join(".git").exists()
        {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(DescriptorBuildError::new(
                "E1902",
                format!(
                    "Descriptor build requires a project root marker (packages.tdm, taida.toml, or .git) above '{}'.",
                    start.display()
                ),
            ));
        }
    }
}

pub(crate) fn field<'a>(fields: &'a [BuchiField], name: &str) -> Option<&'a Expr> {
    fields
        .iter()
        .find_map(|field| (field.name == name).then_some(&field.value))
}

pub(crate) fn required_string_field(
    fields: &[BuchiField],
    field_name: &str,
    descriptor: &str,
) -> Result<String, DescriptorBuildError> {
    match field(fields, field_name) {
        Some(Expr::StringLit(value, _)) => Ok(value.clone()),
        Some(_) => Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "{}.{} must be a string literal in descriptor build mode.",
                descriptor, field_name
            ),
        )),
        None => Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "{} requires a '{}' field in descriptor build mode.",
                descriptor, field_name
            ),
        )),
    }
}

pub(crate) fn optional_string_field(
    fields: &[BuchiField],
    field_name: &str,
    descriptor: &str,
) -> Result<Option<String>, DescriptorBuildError> {
    match field(fields, field_name) {
        Some(Expr::StringLit(value, _)) => Ok(Some(value.clone())),
        Some(_) => Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "{}.{} must be a string literal in descriptor build mode.",
                descriptor, field_name
            ),
        )),
        None => Ok(None),
    }
}

pub(crate) fn required_ident_field(
    fields: &[BuchiField],
    field_name: &str,
    descriptor: &str,
) -> Result<String, DescriptorBuildError> {
    match field(fields, field_name) {
        Some(Expr::Ident(value, _)) => Ok(value.clone()),
        Some(_) => Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "{}.{} must be a symbol reference in descriptor build mode.",
                descriptor, field_name
            ),
        )),
        None => Err(DescriptorBuildError::new(
            "E1902",
            format!("{} requires a '{}' field.", descriptor, field_name),
        )),
    }
}

/// Reject a field that is documented historically but no longer supported.
/// Silent-ignore would let docs / artifact-map / actual output drift.
pub(crate) fn reject_retired_field(
    fields: &[BuchiField],
    field_name: &str,
    descriptor: &str,
    rationale: &str,
) -> Result<(), DescriptorBuildError> {
    if field(fields, field_name).is_some() {
        return Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "{}.{} is not supported in descriptor build mode. {}",
                descriptor, field_name, rationale
            ),
        ));
    }
    Ok(())
}

pub(crate) fn list_expr<'a>(
    fields: &'a [BuchiField],
    field_name: &str,
) -> Result<&'a [Expr], DescriptorBuildError> {
    match field(fields, field_name) {
        Some(Expr::ListLit(items, _)) => Ok(items),
        Some(_) => Err(DescriptorBuildError::new(
            "E1902",
            format!("Descriptor field '{}' must be a list literal.", field_name),
        )),
        None => Ok(&[]),
    }
}

pub(crate) fn ident_list_field(
    fields: &[BuchiField],
    field_name: &str,
) -> Result<Vec<String>, DescriptorBuildError> {
    let mut out = Vec::new();
    for item in list_expr(fields, field_name)? {
        match item {
            Expr::Ident(name, _) => out.push(name.clone()),
            Expr::TypeInst(type_name, hook_fields, _) if type_name == "BuildHook" => {
                out.push(required_string_field(hook_fields, "name", "BuildHook")?);
            }
            _ => {
                return Err(DescriptorBuildError::new(
                    "E1902",
                    format!(
                        "Descriptor field '{}' must contain symbol references.",
                        field_name
                    ),
                ));
            }
        }
    }
    Ok(out)
}

pub(crate) fn string_list_field(
    fields: &[BuchiField],
    field_name: &str,
) -> Result<Vec<String>, DescriptorBuildError> {
    let mut out = Vec::new();
    for item in list_expr(fields, field_name)? {
        match item {
            Expr::StringLit(value, _) => out.push(value.clone()),
            _ => {
                return Err(DescriptorBuildError::new(
                    "E1902",
                    format!(
                        "Descriptor field '{}' must contain string literals.",
                        field_name
                    ),
                ));
            }
        }
    }
    Ok(out)
}

pub(crate) fn env_list_field(
    fields: &[BuchiField],
    field_name: &str,
) -> Result<Vec<(String, String)>, DescriptorBuildError> {
    let mut out = Vec::new();
    for item in list_expr(fields, field_name)? {
        let env_fields = match item {
            Expr::BuchiPack(fields, _) | Expr::TypeInst(_, fields, _) => fields,
            _ => {
                return Err(DescriptorBuildError::new(
                    "E1902",
                    format!(
                        "Descriptor field '{}' must contain @(name, value) packs.",
                        field_name
                    ),
                ));
            }
        };
        out.push((
            required_string_field(env_fields, "name", "BuildHook.env")?,
            required_string_field(env_fields, "value", "BuildHook.env")?,
        ));
    }
    Ok(out)
}

pub(crate) fn resolve_descriptor_imports(
    entry_path: &Path,
    program: &Program,
) -> HashMap<String, PathBuf> {
    let mut symbols = HashMap::new();
    for stmt in &program.statements {
        if let Statement::Import(import) = stmt
            && let Some(path) = module_graph::resolve_local_import_from(entry_path, &import.path)
        {
            let resolved = path.canonicalize().unwrap_or(path);
            for symbol in &import.symbols {
                symbols.insert(
                    symbol.alias.clone().unwrap_or_else(|| symbol.name.clone()),
                    resolved.clone(),
                );
            }
        }
    }
    symbols
}

/// Descriptor names are used directly as staging path segments, artifact-map
/// keys, and hook-log directories. Keep them to one portable path segment:
/// reject traversal, hidden segments, NUL, common Unicode slash/dot lookalikes,
/// and Windows device names before any filesystem write is planned.
pub(crate) fn validate_descriptor_name(name: &str, kind: &str) -> Result<(), DescriptorBuildError> {
    if name.is_empty() {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!("{} name must not be empty.", kind),
        ));
    }
    if name == "." || name == ".." {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!("{} name '{}' is not a valid path segment.", kind, name),
        ));
    }
    if name.starts_with('.') {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!(
                "{} name '{}' must not start with '.' (hidden segments are rejected).",
                kind, name
            ),
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!(
                "{} name '{}' must be a single path segment (no '/' or '\\\\').",
                kind, name
            ),
        ));
    }
    const CONFUSABLE_PATH_CHARS: &[char] = &[
        '\u{2215}', // ∕ DIVISION SLASH
        '\u{2044}', // ⁄ FRACTION SLASH
        '\u{29F8}', // ⧸ BIG SOLIDUS
        '\u{FF0F}', // ／ FULLWIDTH SOLIDUS
        '\u{2024}', // ․ ONE DOT LEADER
        '\u{FF0E}', // ． FULLWIDTH FULL STOP
    ];
    if name.chars().any(|ch| CONFUSABLE_PATH_CHARS.contains(&ch)) {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!(
                "{} name '{}' must not contain look-alike path separator characters.",
                kind, name
            ),
        ));
    }
    let windows_base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(
        windows_base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if reserved {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!(
                "{} name '{}' is reserved on Windows and cannot be used as an artifact path segment.",
                kind, name
            ),
        ));
    }
    if name.contains('\0') {
        return Err(DescriptorBuildError::new(
            "E1916",
            format!("{} name must not contain a NUL byte.", kind),
        ));
    }
    Ok(())
}

pub(crate) fn parse_route_asset(expr: &Expr) -> Result<RouteAssetDescriptor, DescriptorBuildError> {
    let fields = match expr {
        Expr::TypeInst(type_name, fields, _) if type_name == "RouteAsset" => fields,
        _ => {
            return Err(DescriptorBuildError::new(
                "E1902",
                "BuildUnit.assets must contain RouteAsset(...) values.",
            ));
        }
    };
    let path = required_string_field(fields, "path", "RouteAsset")?;
    let unit_symbol = match field(fields, "unit") {
        Some(Expr::Ident(name, _)) => Some(name.clone()),
        Some(_) => {
            return Err(DescriptorBuildError::new(
                "E1902",
                "RouteAsset.unit must be a BuildUnit symbol reference.",
            ));
        }
        None => None,
    };
    let asset_symbol = match field(fields, "asset") {
        Some(Expr::Ident(name, _)) => Some(name.clone()),
        Some(_) => {
            return Err(DescriptorBuildError::new(
                "E1902",
                "RouteAsset.asset must be an AssetBundle symbol reference.",
            ));
        }
        None => None,
    };
    if unit_symbol.is_some() == asset_symbol.is_some() {
        return Err(DescriptorBuildError::new(
            "E1902",
            "RouteAsset requires exactly one of 'unit' or 'asset'.",
        ));
    }
    Ok(RouteAssetDescriptor {
        path,
        unit_symbol,
        asset_symbol,
        name: optional_string_field(fields, "name", "RouteAsset")?,
    })
}

pub(crate) fn parse_build_unit(
    symbol: &str,
    fields: &[BuchiField],
    import_symbols: &HashMap<String, PathBuf>,
) -> Result<BuildUnitDescriptor, DescriptorBuildError> {
    let name = descriptor_name_from_fields(fields, symbol);
    validate_descriptor_name(&name, "BuildUnit")?;
    let target_raw = required_string_field(fields, "target", "BuildUnit")?;
    let target = BuildTarget::parse(&target_raw).ok_or_else(|| {
        DescriptorBuildError::new(
            "E1902",
            format!("BuildUnit '{}' has unknown target '{}'.", name, target_raw),
        )
    })?;
    let entry_symbol = required_ident_field(fields, "entry", "BuildUnit")?;
    let entry_path = import_symbols.get(&entry_symbol).cloned();
    let handler = optional_string_field(fields, "handler", "BuildUnit")?;
    if handler.is_some() && !target.supports_handler() {
        return Err(DescriptorBuildError::new(
            "E1902",
            format!(
                "BuildUnit '{}' uses handler mode, but handler is only valid for Native/WASM targets.",
                name
            ),
        ));
    }
    reject_retired_field(
        fields,
        "output",
        "BuildUnit",
        "Output paths are derived from `<target>/<unit-name>/<entry-stem>` and must not be overridden.",
    )?;
    let mut route_assets = Vec::new();
    for item in list_expr(fields, "assets")? {
        route_assets.push(parse_route_asset(item)?);
    }
    Ok(BuildUnitDescriptor {
        symbol: symbol.to_string(),
        name,
        target,
        entry_symbol,
        entry_path,
        handler,
        route_assets,
        before_hooks: ident_list_field(fields, "before")?,
    })
}

pub(crate) fn parse_build_plan(
    symbol: &str,
    fields: &[BuchiField],
) -> Result<BuildPlanDescriptor, DescriptorBuildError> {
    let name = descriptor_name_from_fields(fields, symbol);
    validate_descriptor_name(&name, "BuildPlan")?;
    Ok(BuildPlanDescriptor {
        symbol: symbol.to_string(),
        name,
        unit_symbols: ident_list_field(fields, "units")?,
        asset_symbols: ident_list_field(fields, "assets")?,
        before_hooks: ident_list_field(fields, "before")?,
    })
}

pub(crate) fn parse_asset_bundle(
    symbol: &str,
    fields: &[BuchiField],
) -> Result<AssetBundleDescriptor, DescriptorBuildError> {
    let name = descriptor_name_from_fields(fields, symbol);
    validate_descriptor_name(&name, "AssetBundle")?;
    Ok(AssetBundleDescriptor {
        symbol: symbol.to_string(),
        name,
        root: required_string_field(fields, "root", "AssetBundle")?,
        files: string_list_field(fields, "files")?,
        output: optional_string_field(fields, "output", "AssetBundle")?,
        before_hooks: ident_list_field(fields, "before")?,
    })
}

pub(crate) fn parse_build_hook(
    symbol: &str,
    fields: &[BuchiField],
) -> Result<BuildHookDescriptor, DescriptorBuildError> {
    let name = descriptor_name_from_fields(fields, symbol);
    validate_descriptor_name(&name, "BuildHook")?;
    Ok(BuildHookDescriptor {
        symbol: symbol.to_string(),
        name,
        command: required_string_field(fields, "command", "BuildHook")?,
        cwd: required_string_field(fields, "cwd", "BuildHook")?,
        env: env_list_field(fields, "env")?,
    })
}

pub(crate) fn build_descriptor_model(
    entry_path: &Path,
    program: &Program,
) -> Result<BuildDescriptorModel, DescriptorBuildError> {
    let import_symbols = resolve_descriptor_imports(entry_path, program);
    let mut model = BuildDescriptorModel::default();
    for stmt in &program.statements {
        match stmt {
            Statement::Assignment(assignment) => match &assignment.value {
                Expr::TypeInst(type_name, fields, _) if type_name == "BuildUnit" => {
                    let unit = parse_build_unit(&assignment.target, fields, &import_symbols)?;
                    if let Some(prev) = model.units_by_symbol.get(&unit.symbol) {
                        return Err(duplicate_descriptor_symbol(
                            "BuildUnit",
                            &unit.symbol,
                            &prev.name,
                            &unit.name,
                        ));
                    }
                    if let Some(prev_symbol) = model
                        .unit_symbol_by_name
                        .insert(unit.name.clone(), unit.symbol.clone())
                    {
                        return Err(duplicate_descriptor_name(
                            "BuildUnit",
                            &unit.name,
                            &prev_symbol,
                            &unit.symbol,
                        ));
                    }
                    model.units_by_symbol.insert(unit.symbol.clone(), unit);
                }
                Expr::TypeInst(type_name, fields, _) if type_name == "BuildPlan" => {
                    let plan = parse_build_plan(&assignment.target, fields)?;
                    if let Some(prev) = model.plans_by_symbol.get(&plan.symbol) {
                        return Err(duplicate_descriptor_symbol(
                            "BuildPlan",
                            &plan.symbol,
                            &prev.name,
                            &plan.name,
                        ));
                    }
                    if let Some(prev_symbol) = model
                        .plan_symbol_by_name
                        .insert(plan.name.clone(), plan.symbol.clone())
                    {
                        return Err(duplicate_descriptor_name(
                            "BuildPlan",
                            &plan.name,
                            &prev_symbol,
                            &plan.symbol,
                        ));
                    }
                    model.plans_by_symbol.insert(plan.symbol.clone(), plan);
                }
                Expr::TypeInst(type_name, fields, _) if type_name == "AssetBundle" => {
                    let asset = parse_asset_bundle(&assignment.target, fields)?;
                    if let Some(prev) = model.assets_by_symbol.get(&asset.symbol) {
                        return Err(duplicate_descriptor_symbol(
                            "AssetBundle",
                            &asset.symbol,
                            &prev.name,
                            &asset.name,
                        ));
                    }
                    if let Some(prev_symbol) = model
                        .asset_symbol_by_name
                        .insert(asset.name.clone(), asset.symbol.clone())
                    {
                        return Err(duplicate_descriptor_name(
                            "AssetBundle",
                            &asset.name,
                            &prev_symbol,
                            &asset.symbol,
                        ));
                    }
                    model.assets_by_symbol.insert(asset.symbol.clone(), asset);
                }
                Expr::TypeInst(type_name, fields, _) if type_name == "BuildHook" => {
                    let hook = parse_build_hook(&assignment.target, fields)?;
                    if let Some(prev) = model.hooks_by_symbol.get(&hook.symbol) {
                        return Err(duplicate_descriptor_symbol(
                            "BuildHook",
                            &hook.symbol,
                            &prev.name,
                            &hook.name,
                        ));
                    }
                    if let Some(prev_symbol) = model
                        .hook_symbol_by_name
                        .insert(hook.name.clone(), hook.symbol.clone())
                    {
                        return Err(duplicate_descriptor_name(
                            "BuildHook",
                            &hook.name,
                            &prev_symbol,
                            &hook.symbol,
                        ));
                    }
                    model.hooks_by_symbol.insert(hook.symbol.clone(), hook);
                }
                _ => {}
            },
            Statement::Export(export) => {
                for symbol in &export.symbols {
                    model.exported_symbols.insert(symbol.clone());
                }
            }
            _ => {}
        }
    }
    Ok(model)
}

/// Descriptor `name` collisions across two different symbols are a silent
/// foot-gun — `taida build --unit X` would resolve to whichever was inserted
/// last. Reject the second occurrence with `[E1902]` so users see the
/// conflict instead of guessing which definition won.
pub(crate) fn duplicate_descriptor_name(
    descriptor: &str,
    name: &str,
    first_symbol: &str,
    second_symbol: &str,
) -> DescriptorBuildError {
    DescriptorBuildError::new(
        "E1902",
        format!(
            "{descriptor} name '{name}' is declared more than once (symbols '{first_symbol}' and '{second_symbol}'); each {descriptor} name must be unique within a descriptor file."
        ),
    )
}

/// Descriptor `symbol` collisions are equally dangerous: rebinding the same
/// Taida-side identifier to two different descriptor instances silently
/// overwrites `*_by_symbol` while leaving the previous `*_symbol_by_name`
/// entry as a stale alias, so `taida build --unit <prev_name>` resolves to
/// the second descriptor. Reject the second binding with `[E1902]` before
/// the overwrite happens.
pub(crate) fn duplicate_descriptor_symbol(
    descriptor: &str,
    symbol: &str,
    first_name: &str,
    second_name: &str,
) -> DescriptorBuildError {
    DescriptorBuildError::new(
        "E1902",
        format!(
            "{descriptor} symbol '{symbol}' is bound more than once (names '{first_name}' and '{second_name}'); each {descriptor} symbol must be defined exactly once within a descriptor file."
        ),
    )
}

pub(crate) fn descriptor_exported_unit_names(model: &BuildDescriptorModel) -> Vec<String> {
    let mut out = model
        .exported_symbols
        .iter()
        .filter_map(|symbol| model.units_by_symbol.get(symbol))
        .map(|unit| unit.name.clone())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

pub(crate) fn descriptor_exported_plan_names(model: &BuildDescriptorModel) -> Vec<String> {
    let mut out = model
        .exported_symbols
        .iter()
        .filter_map(|symbol| model.plans_by_symbol.get(symbol))
        .map(|plan| plan.name.clone())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

pub(crate) fn descriptor_selected_units(
    model: &BuildDescriptorModel,
    selector: &DescriptorBuildSelector,
) -> Result<(Vec<String>, Vec<String>), DescriptorBuildError> {
    let exported_units = descriptor_exported_unit_names(model);
    let exported_plans = descriptor_exported_plan_names(model);
    if exported_units.is_empty() && exported_plans.is_empty() {
        return Err(DescriptorBuildError::new(
            "E1902",
            "Descriptor build mode requested, but the input exports no BuildUnit or BuildPlan descriptors.",
        )
        .suggestion(
            "Export a BuildUnit/BuildPlan symbol, or run single-target build without --unit/--plan/--all-units.",
        ));
    }

    match selector {
        DescriptorBuildSelector::Unit(name) => {
            let Some(symbol) = model.unit_symbol_by_name.get(name) else {
                return Err(DescriptorBuildError::new(
                    "E1903",
                    format!(
                        "No exported BuildUnit named '{}'. Candidates: {}.",
                        name,
                        format_descriptor_candidates(&exported_units)
                    ),
                ));
            };
            if !model.exported_symbols.contains(symbol) {
                return Err(DescriptorBuildError::new(
                    "E1903",
                    format!(
                        "No exported BuildUnit named '{}'. Candidates: {}.",
                        name,
                        format_descriptor_candidates(&exported_units)
                    ),
                ));
            }
            Ok((vec![symbol.clone()], Vec::new()))
        }
        DescriptorBuildSelector::Plan(name) => {
            let Some(symbol) = model.plan_symbol_by_name.get(name) else {
                return Err(DescriptorBuildError::new(
                    "E1904",
                    format!(
                        "No exported BuildPlan named '{}'. Candidates: {}.",
                        name,
                        format_descriptor_candidates(&exported_plans)
                    ),
                ));
            };
            if !model.exported_symbols.contains(symbol) {
                return Err(DescriptorBuildError::new(
                    "E1904",
                    format!(
                        "No exported BuildPlan named '{}'. Candidates: {}.",
                        name,
                        format_descriptor_candidates(&exported_plans)
                    ),
                ));
            }
            let plan = model.plans_by_symbol.get(symbol).expect("plan exists");
            Ok((plan.unit_symbols.clone(), plan.asset_symbols.clone()))
        }
        DescriptorBuildSelector::AllUnits => {
            let mut symbols = model
                .exported_symbols
                .iter()
                .filter(|symbol| model.units_by_symbol.contains_key(*symbol))
                .cloned()
                .collect::<Vec<_>>();
            symbols.sort();
            if symbols.is_empty() {
                return Err(DescriptorBuildError::new(
                    "E1903",
                    "Descriptor build mode requested --all-units, but no BuildUnit descriptors are exported.",
                ));
            }
            Ok((symbols, Vec::new()))
        }
    }
}

pub(crate) fn collect_unit_dependencies(
    model: &BuildDescriptorModel,
    unit_symbol: &str,
) -> Vec<String> {
    let Some(unit) = model.units_by_symbol.get(unit_symbol) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for route in &unit.route_assets {
        if let Some(dep) = route.unit_symbol.as_ref()
            && !deps.contains(dep)
        {
            deps.push(dep.clone());
        }
    }
    deps
}

pub(crate) fn visit_unit_order(
    model: &BuildDescriptorModel,
    symbol: &str,
    visiting: &mut Vec<String>,
    visited: &mut HashSet<String>,
    out: &mut Vec<String>,
) -> Result<(), DescriptorBuildError> {
    if visited.contains(symbol) {
        return Ok(());
    }
    if let Some(pos) = visiting.iter().position(|s| s == symbol) {
        let mut cycle = visiting[pos..].to_vec();
        cycle.push(symbol.to_string());
        let names = cycle
            .iter()
            .map(|s| {
                model
                    .units_by_symbol
                    .get(s)
                    .map(|u| u.name.clone())
                    .unwrap_or_else(|| s.clone())
            })
            .collect::<Vec<_>>();
        return Err(DescriptorBuildError::new(
            "E1940",
            format!(
                "Artifact dependency cycle detected: {}.",
                names.join(" -> ")
            ),
        )
        .context(BuildDiagContext {
            edge_kind: Some("ArtifactDependency"),
            dependency_path: names,
            ..BuildDiagContext::default()
        }));
    }
    let Some(unit) = model.units_by_symbol.get(symbol) else {
        return Err(DescriptorBuildError::new(
            "E1903",
            format!("Unknown BuildUnit symbol '{}'.", symbol),
        ));
    };
    visiting.push(symbol.to_string());
    for dep in collect_unit_dependencies(model, symbol) {
        if !model.units_by_symbol.contains_key(&dep) {
            return Err(DescriptorBuildError::new(
                "E1903",
                format!(
                    "BuildUnit '{}' references unknown artifact dependency '{}'.",
                    unit.name, dep
                ),
            )
            .context(BuildDiagContext {
                unit: Some(unit.name.clone()),
                target: Some(unit.target.as_str().to_string()),
                edge_kind: Some("ArtifactDependency"),
                dependency_path: vec![unit.name.clone(), dep],
                ..BuildDiagContext::default()
            }));
        }
        visit_unit_order(model, &dep, visiting, visited, out)?;
    }
    visiting.pop();
    visited.insert(symbol.to_string());
    out.push(symbol.to_string());
    Ok(())
}

pub(crate) fn descriptor_build_order(
    model: &BuildDescriptorModel,
    roots: &[String],
) -> Result<Vec<String>, DescriptorBuildError> {
    let mut visited = HashSet::new();
    let mut visiting = Vec::new();
    let mut out = Vec::new();
    for symbol in roots {
        visit_unit_order(model, symbol, &mut visiting, &mut visited, &mut out)?;
    }
    Ok(out)
}

pub(crate) fn validate_route_paths(unit: &BuildUnitDescriptor) -> Result<(), DescriptorBuildError> {
    let mut seen = HashSet::<String>::new();
    for route in &unit.route_assets {
        if !route.path.starts_with('/') {
            return Err(DescriptorBuildError::new(
                "E1915",
                format!(
                    "RouteAsset path '{}' in BuildUnit '{}' must start with '/'.",
                    route.path, unit.name
                ),
            )
            .context(BuildDiagContext {
                unit: Some(unit.name.clone()),
                target: Some(unit.target.as_str().to_string()),
                edge_kind: Some("AssetDependency"),
                dependency_path: vec![unit.name.clone(), route.path.clone()],
                ..BuildDiagContext::default()
            }));
        }
        if !seen.insert(route.path.clone()) {
            return Err(DescriptorBuildError::new(
                "E1915",
                format!(
                    "Duplicate RouteAsset path '{}' in BuildUnit '{}'.",
                    route.path, unit.name
                ),
            )
            .context(BuildDiagContext {
                unit: Some(unit.name.clone()),
                target: Some(unit.target.as_str().to_string()),
                edge_kind: Some("AssetDependency"),
                dependency_path: vec![unit.name.clone(), route.path.clone()],
                ..BuildDiagContext::default()
            }));
        }
    }
    Ok(())
}

pub(crate) fn require_build_unit_entry_path(
    unit: &BuildUnitDescriptor,
) -> Result<&Path, DescriptorBuildError> {
    unit.entry_path.as_deref().ok_or_else(|| {
        DescriptorBuildError::new(
            "E1941",
            format!(
                "BuildUnit '{}' entry '{}' must come from a local descriptor import.",
                unit.name, unit.entry_symbol
            ),
        )
        .context(BuildDiagContext {
            unit: Some(unit.name.clone()),
            target: Some(unit.target.as_str().to_string()),
            edge_kind: Some("DescriptorImport"),
            dependency_path: vec![unit.entry_symbol.clone()],
            ..BuildDiagContext::default()
        })
    })
}

pub(crate) fn validate_target_closure(
    unit: &BuildUnitDescriptor,
) -> Result<(), DescriptorBuildError> {
    let entry_path = require_build_unit_entry_path(unit)?;
    let modules = module_graph::collect_local_modules(entry_path).map_err(|err| {
        DescriptorBuildError::new(
            "E1941",
            format!(
                "Cannot compute dependency closure for BuildUnit '{}': {}",
                unit.name, err
            ),
        )
        .context(BuildDiagContext {
            unit: Some(unit.name.clone()),
            target: Some(unit.target.as_str().to_string()),
            edge_kind: Some("NormalImport"),
            dependency_path: vec![entry_path.display().to_string()],
            ..BuildDiagContext::default()
        })
    })?;

    validate_target_closure_modules(unit, entry_path, &modules)
}

pub(crate) fn target_incompatible_import(
    target: BuildTarget,
    import: &ImportStmt,
) -> Option<String> {
    let api = import.path.split('@').next().unwrap_or(&import.path);
    match target {
        BuildTarget::Js | BuildTarget::Native => None,
        BuildTarget::WasmMin => match api {
            "taida-lang/os" | "taida-lang/net" | "taida-lang/terminal" => Some(api.to_string()),
            _ => None,
        },
        BuildTarget::WasmEdge => match api {
            "taida-lang/net" | "taida-lang/terminal" => Some(api.to_string()),
            "taida-lang/os" => first_symbol_not_in(import, &["EnvVar", "allEnv"])
                .map(|symbol| format!("{api}::{symbol}")),
            _ => None,
        },
        BuildTarget::WasmWasi | BuildTarget::WasmFull => match api {
            "taida-lang/net" | "taida-lang/terminal" => Some(api.to_string()),
            "taida-lang/os" => first_symbol_not_in(
                import,
                &[
                    "EnvVar",
                    "allEnv",
                    "Read",
                    "Exists",
                    "writeFile",
                    "readBytesAt",
                ],
            )
            .map(|symbol| format!("{api}::{symbol}")),
            _ => None,
        },
    }
}

pub(crate) fn first_symbol_not_in(import: &ImportStmt, allowed: &[&str]) -> Option<String> {
    if import.symbols.is_empty() {
        return Some("*".to_string());
    }
    import
        .symbols
        .iter()
        .find(|symbol| !allowed.contains(&symbol.name.as_str()))
        .map(|symbol| symbol.name.clone())
}

pub(crate) fn path_has_parent_component(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

pub(crate) fn validate_project_relative_path(
    raw: &str,
    code: &'static str,
    what: &str,
) -> Result<(), DescriptorBuildError> {
    if raw.starts_with('~') || Path::new(raw).is_absolute() || path_has_parent_component(raw) {
        return Err(DescriptorBuildError::new(
            code,
            format!(
                "{} must be project-root-relative without absolute, '~', or '..' segments: '{}'.",
                what, raw
            ),
        ));
    }
    Ok(())
}

pub(crate) fn is_hidden_rel_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|s| s.starts_with('.') && s != "." && s != "..")
            .unwrap_or(false)
    })
}

pub(crate) fn glob_segment_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == t[ti] || p[pi] == b'?') {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            star_ti = ti;
        } else if let Some(star_pi) = star {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

pub(crate) fn glob_path_match(pattern: &str, rel: &Path) -> bool {
    let rel_text = rel.to_string_lossy().replace('\\', "/");
    let path_parts = rel_text.split('/').collect::<Vec<_>>();
    let pat_parts = pattern.split('/').collect::<Vec<_>>();
    fn rec(pat: &[&str], path: &[&str]) -> bool {
        if pat.is_empty() {
            return path.is_empty();
        }
        if pat[0] == "**" {
            if rec(&pat[1..], path) {
                return true;
            }
            return !path.is_empty() && rec(pat, &path[1..]);
        }
        !path.is_empty() && glob_segment_match(pat[0], path[0]) && rec(&pat[1..], &path[1..])
    }
    rec(&pat_parts, &path_parts)
}

pub(crate) fn collect_regular_files_under(
    root: &Path,
) -> Result<Vec<PathBuf>, DescriptorBuildError> {
    let mut out = Vec::new();
    let entries = fs::read_dir(root).map_err(|e| {
        DescriptorBuildError::new(
            "E1912",
            format!("Cannot read AssetBundle root '{}': {}", root.display(), e),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            DescriptorBuildError::new(
                "E1912",
                format!(
                    "Cannot read AssetBundle entry under '{}': {}",
                    root.display(),
                    e
                ),
            )
        })?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| {
            DescriptorBuildError::new(
                "E1913",
                format!(
                    "Cannot inspect AssetBundle entry '{}': {}",
                    path.display(),
                    e
                ),
            )
        })?;
        let ft = meta.file_type();
        if ft.is_symlink() {
            return Err(DescriptorBuildError::new(
                "E1913",
                format!(
                    "AssetBundle entry '{}' is a symlink; symlinks are not followed.",
                    path.display()
                ),
            ));
        }
        if ft.is_dir() {
            out.extend(collect_regular_files_under(&path)?);
        } else if ft.is_file() {
            out.push(path);
        } else {
            return Err(DescriptorBuildError::new(
                "E1913",
                format!(
                    "AssetBundle entry '{}' is not a regular file.",
                    path.display()
                ),
            ));
        }
    }
    out.sort();
    Ok(out)
}

pub(crate) fn asset_output_base(
    asset: &AssetBundleDescriptor,
) -> Result<PathBuf, DescriptorBuildError> {
    let default_output = format!("assets/{}", asset.name);
    let raw = asset.output.as_deref().unwrap_or(&default_output);
    validate_project_relative_path(raw, "E1914", "AssetBundle.output")?;
    Ok(PathBuf::from(raw))
}

pub(crate) fn plan_asset_copies(
    asset: &AssetBundleDescriptor,
    project_root: &Path,
) -> Result<Vec<AssetCopyRecord>, DescriptorBuildError> {
    validate_project_relative_path(&asset.root, "E1910", "AssetBundle.root")?;
    let root_path = project_root.join(&asset.root);
    let root_canon = root_path.canonicalize().map_err(|e| {
        DescriptorBuildError::new(
            "E1910",
            format!(
                "Cannot canonicalize AssetBundle root '{}': {}",
                asset.root, e
            ),
        )
    })?;
    let project_canon = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    if !root_canon.starts_with(&project_canon) {
        return Err(DescriptorBuildError::new(
            "E1910",
            format!(
                "AssetBundle '{}' root escapes the project root: '{}'.",
                asset.name, asset.root
            ),
        ));
    }
    if asset.files.is_empty() {
        return Err(DescriptorBuildError::new(
            "E1911",
            format!(
                "AssetBundle '{}' requires at least one files glob.",
                asset.name
            ),
        ));
    }

    let all_files = collect_regular_files_under(&root_canon)?;
    let output_base = asset_output_base(asset)?;
    let mut records = Vec::new();
    let mut seen_output = HashMap::<PathBuf, PathBuf>::new();
    for pattern in &asset.files {
        validate_project_relative_path(pattern, "E1911", "AssetBundle.files glob")?;
        let include_hidden = pattern.contains("/.") || pattern.starts_with('.');
        for source in &all_files {
            let rel = source.strip_prefix(&root_canon).unwrap_or(source);
            if !include_hidden && is_hidden_rel_path(rel) {
                continue;
            }
            if !glob_path_match(pattern, rel) {
                continue;
            }
            let source_canon = source.canonicalize().map_err(|e| {
                DescriptorBuildError::new(
                    "E1912",
                    format!(
                        "Cannot canonicalize AssetBundle source '{}': {}",
                        source.display(),
                        e
                    ),
                )
            })?;
            if !source_canon.starts_with(&root_canon) {
                return Err(DescriptorBuildError::new(
                    "E1912",
                    format!(
                        "AssetBundle '{}' source escapes root: '{}'.",
                        asset.name,
                        rel.display()
                    ),
                ));
            }
            let output_rel = output_base.join(rel);
            if let Some(previous) = seen_output.insert(output_rel.clone(), source.clone())
                && previous != *source
            {
                return Err(DescriptorBuildError::new(
                    "E1914",
                    format!(
                        "AssetBundle '{}' has duplicate normalized output path '{}'.",
                        asset.name,
                        output_rel.display()
                    ),
                ));
            }
            if !records
                .iter()
                .any(|r: &AssetCopyRecord| r.source == *source && r.output_rel == output_rel)
            {
                records.push(AssetCopyRecord {
                    bundle: asset.name.clone(),
                    source: source.clone(),
                    output_rel,
                });
            }
        }
    }
    records.sort_by(|a, b| a.output_rel.cmp(&b.output_rel));
    Ok(records)
}

pub(crate) fn copy_asset_bundle_to_stage(
    asset: &AssetBundleDescriptor,
    project_root: &Path,
    tx: &DescriptorTransaction,
    owner_unit: Option<&BuildUnitDescriptor>,
) -> Result<CopiedAssetBundleRecord, DescriptorBuildError> {
    let records = plan_asset_copies(asset, project_root).map_err(|err| {
        let dependency_path = owner_unit
            .map(|unit| vec![unit.name.clone(), asset.name.clone()])
            .unwrap_or_else(|| vec![asset.name.clone()]);
        err.context(BuildDiagContext {
            unit: owner_unit.map(|unit| unit.name.clone()),
            target: owner_unit.map(|unit| unit.target.as_str().to_string()),
            edge_kind: Some("AssetDependency"),
            dependency_path,
            transaction_id: Some(tx.id.clone()),
            ..BuildDiagContext::default()
        })
    })?;
    let output_base = asset_output_base(asset)?;
    for record in &records {
        let dest = tx.staging_root.join(&record.output_rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                DescriptorBuildError::new(
                    "E1912",
                    format!(
                        "Cannot create staged asset directory '{}': {}",
                        parent.display(),
                        e
                    ),
                )
            })?;
        }
        fs::copy(&record.source, &dest).map_err(|e| {
            DescriptorBuildError::new(
                "E1912",
                format!(
                    "Cannot copy AssetBundle '{}' source '{}' to staging: {}",
                    asset.name,
                    record.source.display(),
                    e
                ),
            )
        })?;
    }
    let sidecar_dir = tx.staging_root.join(asset_output_base(asset)?);
    fs::create_dir_all(&sidecar_dir).map_err(|e| {
        DescriptorBuildError::new(
            "E1912",
            format!(
                "Cannot create AssetBundle transaction sidecar directory '{}': {}",
                sidecar_dir.display(),
                e
            ),
        )
    })?;
    fs::write(sidecar_dir.join(".transaction-id"), &tx.id).map_err(|e| {
        DescriptorBuildError::new(
            "E1912",
            format!(
                "Cannot write AssetBundle transaction sidecar '{}': {}",
                sidecar_dir.join(".transaction-id").display(),
                e
            ),
        )
    })?;
    Ok(CopiedAssetBundleRecord {
        name: asset.name.clone(),
        output_rel: output_base,
        files: records,
    })
}

pub(crate) fn validate_asset_output_collision(
    asset: &AssetBundleDescriptor,
    seen_outputs: &mut HashMap<PathBuf, String>,
    tx: &DescriptorTransaction,
) -> Result<(), DescriptorBuildError> {
    let output = asset_output_base(asset)?;
    if let Some(previous) = seen_outputs.insert(output.clone(), asset.name.clone())
        && previous != asset.name
    {
        return Err(DescriptorBuildError::new(
            "E1914",
            format!(
                "AssetBundle '{}' collides with AssetBundle '{}' at output '{}'.",
                asset.name,
                previous,
                output.display()
            ),
        )
        .context(BuildDiagContext {
            edge_kind: Some("AssetDependency"),
            dependency_path: vec![previous, asset.name.clone()],
            transaction_id: Some(tx.id.clone()),
            ..BuildDiagContext::default()
        }));
    }
    Ok(())
}

pub(crate) fn descriptor_shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

pub(crate) fn descriptor_hook_fingerprint(hook: &BuildHookDescriptor) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(hook.name.as_bytes());
    input.push(0);
    input.extend_from_slice(hook.cwd.as_bytes());
    input.push(0);
    input.extend_from_slice(hook.command.as_bytes());
    input.push(0);
    for (name, value) in &hook.env {
        input.extend_from_slice(name.as_bytes());
        input.push(b'=');
        input.extend_from_slice(value.as_bytes());
        input.push(0);
    }
    format!("sha256:{}", taida::crypto::sha256_hex_bytes(&input))
}

pub(crate) fn run_descriptor_hook(
    hook: &BuildHookDescriptor,
    project_root: &Path,
    tx: &DescriptorTransaction,
    run_hooks: bool,
    records: &mut DescriptorBuildRecords,
) -> Result<(), DescriptorBuildError> {
    if !run_hooks {
        return Err(DescriptorBuildError::new(
            "E1951",
            format!(
                "BuildHook '{}' is attached but hooks are disabled by default.",
                hook.name
            ),
        )
        .suggestion("Pass `--run-hooks` to execute BuildHook before hooks.")
        .context(BuildDiagContext {
            hook_name: Some(hook.name.clone()),
            transaction_id: Some(tx.id.clone()),
            ..BuildDiagContext::default()
        }));
    }
    validate_project_relative_path(&hook.cwd, "E1950", "BuildHook.cwd")?;
    let cwd = project_root.join(&hook.cwd);
    let cwd_canon = cwd.canonicalize().map_err(|e| {
        DescriptorBuildError::new(
            "E1950",
            format!("Cannot canonicalize BuildHook cwd '{}': {}", hook.cwd, e),
        )
    })?;
    let project_canon = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    if !cwd_canon.starts_with(&project_canon) {
        return Err(DescriptorBuildError::new(
            "E1950",
            format!("BuildHook '{}' cwd escapes project root.", hook.name),
        ));
    }
    let mut command = descriptor_shell_command(&hook.command);
    command.current_dir(&cwd_canon);
    // Strip the parent environment so descriptor builds stay reproducible:
    // anything the hook needs must come from the descriptor's own `env`
    // declaration. `PATH`, `HOME`, `LANG`, `LC_ALL` are forwarded because
    // shells / locale-aware tools refuse to start without them.
    command.env_clear();
    for forwarded in HOOK_FORWARDED_ENV_VARS {
        if let Some(value) = std::env::var_os(forwarded) {
            command.env(forwarded, value);
        }
    }
    for (name, value) in &hook.env {
        command.env(name, value);
    }
    let output = command.output().map_err(|e| {
        DescriptorBuildError::new(
            "E1952",
            format!("Cannot execute BuildHook '{}': {}", hook.name, e),
        )
    })?;
    let log_dir = tx.build_root.join("hooks").join(&hook.name);
    fs::create_dir_all(&log_dir).map_err(|e| {
        DescriptorBuildError::new(
            "E1952",
            format!(
                "Cannot create BuildHook log directory '{}': {}",
                log_dir.display(),
                e
            ),
        )
    })?;
    let ordinal = records
        .hooks
        .iter()
        .filter(|existing| existing.as_str() == hook.name.as_str())
        .count()
        + 1;
    let log_name = if ordinal == 1 {
        format!("{}.log", tx.id)
    } else {
        format!("{}-{}.log", tx.id, ordinal)
    };
    let log_path = log_dir.join(log_name);
    let tmp_log_path = log_dir.join(format!(
        ".{}.tmp-{}",
        log_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("hook"),
        std::process::id()
    ));
    let fingerprint = descriptor_hook_fingerprint(hook);
    let log = format!(
        "hook={}\ncwd={}\ncommand={}\nenv={}\nfingerprint={}\nstatus={}\nstdout:\n{}\nstderr:\n{}\n",
        hook.name,
        cwd_canon.display(),
        hook.command,
        hook.env
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect::<Vec<_>>()
            .join(","),
        fingerprint,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut log_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_log_path)
        .map_err(|e| {
            DescriptorBuildError::new(
                "E1952",
                format!(
                    "Cannot create BuildHook log temp file '{}': {}",
                    tmp_log_path.display(),
                    e
                ),
            )
        })?;
    use std::io::Write as _;
    if let Err(e) = log_file.write_all(log.as_bytes()) {
        let _ = fs::remove_file(&tmp_log_path);
        return Err(DescriptorBuildError::new(
            "E1952",
            format!(
                "Cannot write BuildHook log temp file '{}': {}",
                tmp_log_path.display(),
                e
            ),
        ));
    }
    drop(log_file);
    if log_path.exists() {
        let _ = fs::remove_file(&tmp_log_path);
        return Err(DescriptorBuildError::new(
            "E1952",
            format!("BuildHook log '{}' already exists.", log_path.display()),
        ));
    }
    fs::rename(&tmp_log_path, &log_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_log_path);
        DescriptorBuildError::new(
            "E1952",
            format!(
                "Cannot commit BuildHook log '{}' from temp file '{}': {}",
                log_path.display(),
                tmp_log_path.display(),
                e
            ),
        )
    })?;
    records.hooks.push(hook.name.clone());
    if !output.status.success() {
        return Err(DescriptorBuildError::new(
            "E1952",
            format!("BuildHook '{}' failed.", hook.name),
        )
        .context(BuildDiagContext {
            hook_name: Some(hook.name.clone()),
            cwd: Some(cwd_canon.display().to_string()),
            exit_code: output.status.code(),
            transaction_id: Some(tx.id.clone()),
            ..BuildDiagContext::default()
        }));
    }
    Ok(())
}

pub(crate) fn run_hooks_by_symbol(
    model: &BuildDescriptorModel,
    hooks: &[String],
    project_root: &Path,
    tx: &DescriptorTransaction,
    run_hooks: bool,
    records: &mut DescriptorBuildRecords,
) -> Result<(), DescriptorBuildError> {
    for hook_symbol in hooks {
        let hook = model.hooks_by_symbol.get(hook_symbol).ok_or_else(|| {
            DescriptorBuildError::new("E1950", format!("Unknown BuildHook '{}'.", hook_symbol))
        })?;
        run_descriptor_hook(hook, project_root, tx, run_hooks, records)?;
    }
    Ok(())
}

pub(crate) fn build_unit_output_path(
    tx: &DescriptorTransaction,
    unit: &BuildUnitDescriptor,
) -> Result<PathBuf, DescriptorBuildError> {
    let entry_path = require_build_unit_entry_path(unit)?;
    let dir = tx.staging_root.join(unit.target.as_str()).join(&unit.name);
    let stem = entry_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&unit.name);
    Ok(match unit.target {
        BuildTarget::Js => dir.join(format!("{}.mjs", stem)),
        BuildTarget::Native => dir.join(&unit.name),
        BuildTarget::WasmMin
        | BuildTarget::WasmWasi
        | BuildTarget::WasmEdge
        | BuildTarget::WasmFull => dir.join(format!("{}.wasm", stem)),
    })
}

pub(crate) fn run_child_build(
    unit: &BuildUnitDescriptor,
    tx: &DescriptorTransaction,
    release_mode: bool,
    no_check: bool,
) -> Result<PathBuf, DescriptorBuildError> {
    let entry_path = require_build_unit_entry_path(unit)?;
    let output_path = build_unit_output_path(tx, unit)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot create staged unit directory '{}': {}",
                    parent.display(),
                    e
                ),
            )
        })?;
    }
    let exe = env::current_exe().map_err(|e| {
        DescriptorBuildError::new(
            "E1923",
            format!("Cannot resolve current taida executable: {}", e),
        )
    })?;
    let mut cmd = Command::new(exe);
    if no_check {
        cmd.arg("--no-check");
    }
    cmd.arg("build")
        .arg(unit.target.as_str())
        .arg(entry_path)
        .arg("-o")
        .arg(&output_path);
    if let Some(handler) = &unit.handler {
        cmd.arg("--handler").arg(handler);
    }
    if release_mode {
        cmd.arg("--release");
    }
    let output = cmd.output().map_err(|e| {
        DescriptorBuildError::new(
            "E1923",
            format!(
                "Cannot spawn child build for BuildUnit '{}': {}",
                unit.name, e
            ),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(DescriptorBuildError::new(
            "E1942",
            format!(
                "BuildUnit '{}' target '{}' failed.\nstdout:\n{}\nstderr:\n{}",
                unit.name,
                unit.target.as_str(),
                stdout,
                stderr
            ),
        )
        .context(BuildDiagContext {
            unit: Some(unit.name.clone()),
            target: Some(unit.target.as_str().to_string()),
            edge_kind: Some("NormalImport"),
            dependency_path: vec![entry_path.display().to_string()],
            transaction_id: Some(tx.id.clone()),
            exit_code: output.status.code(),
            ..BuildDiagContext::default()
        }));
    }
    if let Some(parent) = output_path.parent() {
        fs::write(parent.join(".transaction-id"), &tx.id).map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot write BuildUnit transaction sidecar '{}': {}",
                    parent.join(".transaction-id").display(),
                    e
                ),
            )
        })?;
    }
    Ok(output_path)
}

pub(crate) fn json_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn write_artifact_map(
    tx: &DescriptorTransaction,
    selector: &DescriptorBuildSelector,
    records: &DescriptorBuildRecords,
) -> Result<(), DescriptorBuildError> {
    let units = records
        .units
        .iter()
        .map(|unit| {
            json!({
                "name": unit.name,
                "target": unit.target,
                "entry_symbol": unit.entry_symbol,
                "entry": unit.entry,
                "output": json_path(&unit.output_rel),
                "dependencies": unit.dependencies,
                "route_assets": unit.route_assets.iter().map(|route| json!({
                    "path": route.path,
                    "unit": route.unit_symbol,
                    "asset": route.asset_symbol,
                    "name": route.name,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let assets = records
        .assets
        .iter()
        .map(|asset| {
            json!({
                "name": asset.name,
                "output": json_path(&asset.output_rel),
                "files": asset.files.iter().map(|file| json!({
                    "bundle": file.bundle,
                    "source": json_path(&file.source),
                    "output": json_path(&file.output_rel),
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let selector_json = match selector {
        DescriptorBuildSelector::Unit(name) => json!({"kind": "unit", "name": name}),
        DescriptorBuildSelector::Plan(name) => json!({"kind": "plan", "name": name}),
        DescriptorBuildSelector::AllUnits => json!({"kind": "all-units"}),
    };
    let map = json!({
        "artifact_graph_version": 1,
        "transaction_id": tx.id,
        "committed_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must be after unix epoch")
            .as_secs(),
        "selectors": selector_json,
        "build_mode": "descriptor",
        "units": units,
        "assets": assets,
        "hooks": records.hooks,
    });
    let path = tx.staging_root.join("artifact-map.json");
    fs::write(
        &path,
        serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|e| {
        DescriptorBuildError::new(
            "E1923",
            format!(
                "Cannot write staged artifact-map '{}': {}",
                path.display(),
                e
            ),
        )
    })
}

pub(crate) fn collect_descriptor_replacements(
    tx: &DescriptorTransaction,
    records: &DescriptorBuildRecords,
) -> Vec<(PathBuf, PathBuf)> {
    let mut replacements = Vec::new();
    let mut seen = HashSet::<PathBuf>::new();
    for unit in &records.units {
        let rel = PathBuf::from(&unit.target).join(&unit.name);
        if seen.insert(rel.clone()) {
            replacements.push((tx.staging_root.join(&rel), tx.build_root.join(&rel)));
        }
    }
    for asset in &records.assets {
        let rel = asset.output_rel.clone();
        if seen.insert(rel.clone()) {
            replacements.push((tx.staging_root.join(&rel), tx.build_root.join(&rel)));
        }
    }
    replacements.push((
        tx.staging_root.join("artifact-map.json"),
        tx.build_root.join("artifact-map.json"),
    ));
    replacements
}

pub(crate) fn backup_path_for(tx: &DescriptorTransaction, final_path: &Path) -> PathBuf {
    let rel = final_path
        .strip_prefix(&tx.build_root)
        .unwrap_or(final_path);
    tx.replaced_root.join(rel)
}

pub(crate) fn commit_descriptor_transaction(
    tx: &DescriptorTransaction,
    records: &DescriptorBuildRecords,
) -> Result<(), DescriptorBuildError> {
    let replacements = collect_descriptor_replacements(tx, records);
    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();

    for (_stage, final_path) in &replacements {
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                DescriptorBuildError::new(
                    "E1924",
                    format!(
                        "Cannot create build output directory '{}': {}",
                        parent.display(),
                        e
                    ),
                )
            })?;
        }
        if final_path.exists() {
            let backup = backup_path_for(tx, final_path);
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    DescriptorBuildError::new(
                        "E1924",
                        format!(
                            "Cannot create rollback directory '{}': {}",
                            parent.display(),
                            e
                        ),
                    )
                })?;
            }
            fs::rename(final_path, &backup).map_err(|e| {
                DescriptorBuildError::new(
                    "E1924",
                    format!(
                        "Atomic replace backup failed for '{}': {}",
                        final_path.display(),
                        e
                    ),
                )
            })?;
            backups.push((final_path.clone(), backup));
        }
    }

    let mut committed = Vec::<PathBuf>::new();
    for (stage, final_path) in &replacements {
        if !stage.exists() {
            for path in &committed {
                let _ = if path.is_dir() {
                    fs::remove_dir_all(path)
                } else {
                    fs::remove_file(path)
                };
            }
            for (final_path, backup) in backups.iter().rev() {
                let _ = fs::rename(backup, final_path);
            }
            return Err(DescriptorBuildError::new(
                "E1924",
                format!("Staged output '{}' is missing.", stage.display()),
            ));
        }
        fs::rename(stage, final_path).map_err(|e| {
            for path in &committed {
                let _ = if path.is_dir() {
                    fs::remove_dir_all(path)
                } else {
                    fs::remove_file(path)
                };
            }
            for (final_path, backup) in backups.iter().rev() {
                let _ = fs::rename(backup, final_path);
            }
            DescriptorBuildError::new(
                "E1924",
                format!(
                    "Atomic replace commit failed from '{}' to '{}': {}",
                    stage.display(),
                    final_path.display(),
                    e
                ),
            )
        })?;
        committed.push(final_path.clone());
    }

    for (_final_path, backup) in backups {
        let _ = if backup.is_dir() {
            fs::remove_dir_all(backup)
        } else {
            fs::remove_file(backup)
        };
    }
    tx.cleanup();
    Ok(())
}

pub(crate) fn run_descriptor_build_driver(
    entry_path: &Path,
    program: &Program,
    selector: DescriptorBuildSelector,
    run_hooks: bool,
    release_mode: bool,
    no_check: bool,
) -> Result<DescriptorBuildRecords, DescriptorBuildError> {
    let project_root = descriptor_project_root(entry_path)?;
    let model = build_descriptor_model(entry_path, program)?;
    let (root_units, plan_assets) = descriptor_selected_units(&model, &selector)?;
    let build_order = descriptor_build_order(&model, &root_units)?;
    let tx = DescriptorTransaction::new(&project_root)?;
    let mut records = DescriptorBuildRecords::default();
    let result: Result<(), DescriptorBuildError> = (|| {
        if let DescriptorBuildSelector::Plan(name) = &selector
            && let Some(plan_symbol) = model.plan_symbol_by_name.get(name)
            && let Some(plan) = model.plans_by_symbol.get(plan_symbol)
        {
            run_hooks_by_symbol(
                &model,
                &plan.before_hooks,
                &project_root,
                &tx,
                run_hooks,
                &mut records,
            )?;
        }

        let mut copied_assets = HashSet::<String>::new();
        let mut asset_outputs = HashMap::<PathBuf, String>::new();
        for asset_symbol in plan_assets {
            let asset = model.assets_by_symbol.get(&asset_symbol).ok_or_else(|| {
                DescriptorBuildError::new(
                    "E1910",
                    format!("Unknown AssetBundle '{}'.", asset_symbol),
                )
            })?;
            validate_asset_output_collision(asset, &mut asset_outputs, &tx)?;
            run_hooks_by_symbol(
                &model,
                &asset.before_hooks,
                &project_root,
                &tx,
                run_hooks,
                &mut records,
            )?;
            let copied = copy_asset_bundle_to_stage(asset, &project_root, &tx, None)?;
            copied_assets.insert(asset.symbol.clone());
            records.assets.push(copied);
        }

        for symbol in &build_order {
            let unit = model.units_by_symbol.get(symbol).ok_or_else(|| {
                DescriptorBuildError::new("E1903", format!("Unknown BuildUnit '{}'.", symbol))
            })?;
            validate_route_paths(unit)?;
            validate_target_closure(unit)?;
            run_hooks_by_symbol(
                &model,
                &unit.before_hooks,
                &project_root,
                &tx,
                run_hooks,
                &mut records,
            )?;
            for route in &unit.route_assets {
                if let Some(asset_symbol) = route.asset_symbol.as_ref() {
                    let asset = model.assets_by_symbol.get(asset_symbol).ok_or_else(|| {
                        DescriptorBuildError::new(
                            "E1910",
                            format!(
                                "BuildUnit '{}' references unknown AssetBundle '{}'.",
                                unit.name, asset_symbol
                            ),
                        )
                    })?;
                    if copied_assets.insert(asset.symbol.clone()) {
                        validate_asset_output_collision(asset, &mut asset_outputs, &tx)?;
                        run_hooks_by_symbol(
                            &model,
                            &asset.before_hooks,
                            &project_root,
                            &tx,
                            run_hooks,
                            &mut records,
                        )?;
                        records.assets.push(copy_asset_bundle_to_stage(
                            asset,
                            &project_root,
                            &tx,
                            Some(unit),
                        )?);
                    }
                }
            }
            let output = run_child_build(unit, &tx, release_mode, no_check)?;
            let rel = output
                .strip_prefix(&tx.staging_root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| output.clone());
            let dependencies = collect_unit_dependencies(&model, symbol)
                .into_iter()
                .filter_map(|dep| model.units_by_symbol.get(&dep).map(|u| u.name.clone()))
                .collect::<Vec<_>>();
            records.units.push(BuiltUnitRecord {
                name: unit.name.clone(),
                target: unit.target.as_str().to_string(),
                entry_symbol: unit.entry_symbol.clone(),
                entry: require_build_unit_entry_path(unit)?.display().to_string(),
                output_rel: rel,
                dependencies,
                route_assets: unit.route_assets.clone(),
            });
        }
        write_artifact_map(&tx, &selector, &records)?;
        commit_descriptor_transaction(&tx, &records)?;
        Ok(())
    })();

    match result {
        Ok(()) => Ok(records),
        Err(mut err) => {
            tx.cleanup();
            if err.context.transaction_id.is_none() {
                err.context.transaction_id = Some(tx.id.clone());
            }
            Err(err)
        }
    }
}

pub(crate) fn run_build_descriptor_mode(
    input_path: &Path,
    selector: DescriptorBuildSelector,
    run_hooks: bool,
    release_mode: bool,
    no_check: bool,
    diag_format: DiagFormat,
    compile_stats: &mut CompileDiagStats,
) -> ! {
    let entry_path = match build_descriptor_entry_path(input_path) {
        Ok(path) => path,
        Err(message) => {
            emit_build_cli_diagnostic_and_exit(
                compile_stats,
                diag_format,
                "E1902",
                &message,
                Some("Pass a .td file or a directory containing main.td."),
                1,
            );
        }
    };

    let source = match fs::read_to_string(&entry_path) {
        Ok(source) => source,
        Err(e) => {
            let message = format!("Error reading file '{}': {}", entry_path.display(), e);
            emit_build_cli_diagnostic_and_exit(
                compile_stats,
                diag_format,
                "E1902",
                &message,
                None,
                1,
            );
        }
    };

    let (program, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        for err in &parse_errors {
            if diag_format == DiagFormat::Jsonl {
                let (code, suggestion) = split_diag_code_and_hint(&err.message);
                emit_compile_diag_jsonl(
                    compile_stats,
                    "ERROR",
                    "parse",
                    code,
                    &err.message,
                    Some(&entry_path.to_string_lossy()),
                    Some(err.span.line),
                    Some(err.span.column),
                    suggestion,
                );
            } else {
                eprintln!("{}", err);
            }
        }
        std::process::exit(1);
    }

    match run_descriptor_build_driver(
        &entry_path,
        &program,
        selector,
        run_hooks,
        release_mode,
        no_check,
    ) {
        Ok(records) => {
            if diag_format == DiagFormat::Text {
                println!(
                    "Built descriptor graph: {} unit(s), {} asset bundle(s)",
                    records.units.len(),
                    records.assets.len()
                );
            }
            std::process::exit(0);
        }
        Err(error) => emit_descriptor_build_error_and_exit(error, diag_format, compile_stats),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DescriptorBuildSelector {
    Unit(String),
    Plan(String),
    AllUnits,
}

#[derive(Default)]
pub(crate) struct DescriptorBuildFlags {
    pub(crate) unit: Option<String>,
    pub(crate) plan: Option<String>,
    pub(crate) all_units: bool,
}

impl DescriptorBuildFlags {
    pub(crate) fn selector_count(&self) -> usize {
        usize::from(self.unit.is_some())
            + usize::from(self.plan.is_some())
            + usize::from(self.all_units)
    }

    pub(crate) fn selector(&self) -> Option<DescriptorBuildSelector> {
        match (self.unit.as_ref(), self.plan.as_ref(), self.all_units) {
            (Some(unit), None, false) => Some(DescriptorBuildSelector::Unit(unit.clone())),
            (None, Some(plan), false) => Some(DescriptorBuildSelector::Plan(plan.clone())),
            (None, None, true) => Some(DescriptorBuildSelector::AllUnits),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DescriptorBuildError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    suggestion: Option<String>,
    context: Box<BuildDiagContext>,
    exit_code: i32,
}

impl DescriptorBuildError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            suggestion: None,
            context: Box::default(),
            exit_code: 1,
        }
    }

    fn suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    fn context(mut self, context: BuildDiagContext) -> Self {
        self.context = Box::new(context);
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BuildUnitDescriptor {
    pub(crate) symbol: String,
    pub(crate) name: String,
    pub(crate) target: BuildTarget,
    pub(crate) entry_symbol: String,
    pub(crate) entry_path: Option<PathBuf>,
    pub(crate) handler: Option<String>,
    pub(crate) route_assets: Vec<RouteAssetDescriptor>,
    pub(crate) before_hooks: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildPlanDescriptor {
    symbol: String,
    name: String,
    unit_symbols: Vec<String>,
    asset_symbols: Vec<String>,
    before_hooks: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AssetBundleDescriptor {
    symbol: String,
    name: String,
    root: String,
    files: Vec<String>,
    output: Option<String>,
    before_hooks: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteAssetDescriptor {
    path: String,
    unit_symbol: Option<String>,
    asset_symbol: Option<String>,
    name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildHookDescriptor {
    symbol: String,
    name: String,
    command: String,
    cwd: String,
    env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BuildDescriptorModel {
    units_by_symbol: HashMap<String, BuildUnitDescriptor>,
    unit_symbol_by_name: HashMap<String, String>,
    plans_by_symbol: HashMap<String, BuildPlanDescriptor>,
    plan_symbol_by_name: HashMap<String, String>,
    assets_by_symbol: HashMap<String, AssetBundleDescriptor>,
    asset_symbol_by_name: HashMap<String, String>,
    hooks_by_symbol: HashMap<String, BuildHookDescriptor>,
    /// Tracks BuildHook `name` -> `symbol` so duplicate hook names are
    /// caught alongside BuildUnit / BuildPlan / AssetBundle. Without this
    /// map two `BuildHook(name <= "deploy",...)` definitions silently
    /// coexist in `hooks_by_symbol`, leaving any future name-based CLI or
    /// docs lookup ambiguous.
    hook_symbol_by_name: HashMap<String, String>,
    exported_symbols: HashSet<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct AssetCopyRecord {
    bundle: String,
    source: PathBuf,
    output_rel: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct BuiltUnitRecord {
    name: String,
    target: String,
    entry_symbol: String,
    entry: String,
    output_rel: PathBuf,
    dependencies: Vec<String>,
    route_assets: Vec<RouteAssetDescriptor>,
}

#[derive(Clone, Debug)]
pub(crate) struct CopiedAssetBundleRecord {
    name: String,
    output_rel: PathBuf,
    files: Vec<AssetCopyRecord>,
}

#[derive(Default)]
pub(crate) struct DescriptorBuildRecords {
    units: Vec<BuiltUnitRecord>,
    assets: Vec<CopiedAssetBundleRecord>,
    hooks: Vec<String>,
}

pub(crate) struct DescriptorTransaction {
    id: String,
    build_root: PathBuf,
    staging_root: PathBuf,
    replaced_root: PathBuf,
    _lock: DescriptorBuildLock,
}

pub(crate) struct DescriptorBuildLock {
    path: PathBuf,
    file: Option<fs::File>,
}

impl DescriptorBuildLock {
    fn acquire(build_root: &Path) -> Result<Self, DescriptorBuildError> {
        let path = build_root.join(".lock");
        let pid = std::process::id() as u64;
        let body = || {
            serde_json::to_string_pretty(&json!({
                "pid": pid,
                "created_at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock must be after 1970-01-01 (UNIX epoch)")
                    .as_secs(),
            }))
            .unwrap_or_else(|_| format!("{{\"pid\":{}}}", pid))
        };

        for _attempt in 0..3 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(body().as_bytes()) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(DescriptorBuildError::new(
                            "E1923",
                            format!(
                                "Cannot write descriptor build lock '{}': {}",
                                path.display(),
                                e
                            ),
                        ));
                    }
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let lock_pid = descriptor_lock_pid(&path);
                    if let Some(false) = lock_pid.and_then(descriptor_pid_alive) {
                        let _ = append_descriptor_cleanup_log(
                            build_root,
                            &format!(
                                "remove build-lock={} reason=dead-pid pid={}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or(".lock"),
                                lock_pid
                                    .map(|pid| pid.to_string())
                                    .unwrap_or_else(|| "-".to_string())
                            ),
                        );
                        fs::remove_file(&path).map_err(|remove_err| {
                            DescriptorBuildError::new(
                                "E1923",
                                format!(
                                    "Cannot remove stale descriptor build lock '{}' (pid={}): {}",
                                    path.display(),
                                    lock_pid
                                        .map(|pid| pid.to_string())
                                        .unwrap_or_else(|| "-".to_string()),
                                    remove_err
                                ),
                            )
                        })?;
                        continue;
                    }
                    let owner = lock_pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    return Err(DescriptorBuildError::new(
                        "E1923",
                        format!(
                            "Descriptor build root '{}' is locked by pid {}. Wait for the running descriptor build to finish, or remove '{}' if that process is gone.",
                            build_root.display(),
                            owner,
                            path.display()
                        ),
                    ));
                }
                Err(e) => {
                    return Err(DescriptorBuildError::new(
                        "E1923",
                        format!(
                            "Cannot create descriptor build lock '{}': {}",
                            path.display(),
                            e
                        ),
                    ));
                }
            }
        }

        Err(DescriptorBuildError::new(
            "E1923",
            format!("Cannot acquire descriptor build lock '{}'.", path.display()),
        ))
    }
}

impl Drop for DescriptorBuildLock {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

impl DescriptorTransaction {
    fn new(project_root: &Path) -> Result<Self, DescriptorBuildError> {
        let id = descriptor_transaction_id();
        let build_root = project_root.join(".taida").join("build");
        let staging_root = build_root.join(format!(".tmp-{}", id));
        let replaced_root = staging_root.join(format!(".replaced-{}", id));
        fs::create_dir_all(&build_root).map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot create descriptor build root '{}': {}",
                    build_root.display(),
                    e
                ),
            )
        })?;
        cleanup_stale_descriptor_staging(&build_root)?;
        let lock = DescriptorBuildLock::acquire(&build_root)?;
        fs::create_dir(&staging_root).map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot create descriptor build staging directory '{}': {}",
                    staging_root.display(),
                    e
                ),
            )
        })?;
        fs::create_dir_all(&replaced_root).map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot create descriptor build replacement directory '{}': {}",
                    replaced_root.display(),
                    e
                ),
            )
        })?;
        let tx_json = json!({
            "transaction_id": id,
            "pid": std::process::id(),
            "created_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after 1970-01-01 (UNIX epoch)")
                .as_secs(),
        });
        fs::write(
            staging_root.join("transaction.json"),
            serde_json::to_string_pretty(&tx_json).unwrap_or_else(|_| "{}".to_string()),
        )
        .map_err(|e| {
            DescriptorBuildError::new(
                "E1923",
                format!(
                    "Cannot write descriptor build transaction file '{}': {}",
                    staging_root.join("transaction.json").display(),
                    e
                ),
            )
        })?;
        Ok(Self {
            id,
            build_root,
            staging_root,
            replaced_root,
            _lock: lock,
        })
    }

    fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.staging_root);
    }
}

/// Validate that no module in `modules` imports a target-incompatible API.
/// The inner re-parse defends against the TOCTOU race between
/// `module_graph::collect_local_modules` (which already vets parse errors)
/// and the file being rewritten before this re-read. Splitting it out as a
/// `pub(crate)` helper lets a test inject a parse-broken module path
/// without racing the upstream collector.
pub(crate) fn validate_target_closure_modules(
    unit: &BuildUnitDescriptor,
    entry_path: &Path,
    modules: &[PathBuf],
) -> Result<(), DescriptorBuildError> {
    if matches!(unit.target, BuildTarget::Js | BuildTarget::Native) {
        return Ok(());
    }
    for module in modules {
        let source = fs::read_to_string(module).map_err(|e| {
            DescriptorBuildError::new(
                "E1941",
                format!("Cannot read closure module '{}': {}", module.display(), e),
            )
        })?;
        let (program, parse_errors) = parse(&source);
        if !parse_errors.is_empty() {
            let summary = parse_errors
                .first()
                .map(|e| format!("{e}"))
                .unwrap_or_else(|| String::from("parse error"));
            return Err(DescriptorBuildError::new(
                "E1941",
                format!(
                    "BuildUnit '{}' closure module '{}' has parse errors and cannot be validated against target '{}': {}",
                    unit.name,
                    module.display(),
                    unit.target.as_str(),
                    summary
                ),
            )
            .context(BuildDiagContext {
                unit: Some(unit.name.clone()),
                target: Some(unit.target.as_str().to_string()),
                edge_kind: Some("NormalImport"),
                dependency_path: vec![
                    entry_path.display().to_string(),
                    module.display().to_string(),
                ],
                ..BuildDiagContext::default()
            }));
        }
        for stmt in &program.statements {
            if let Statement::Import(import) = stmt
                && let Some(api) = target_incompatible_import(unit.target, import)
            {
                return Err(DescriptorBuildError::new(
                    "E1941",
                    format!(
                        "BuildUnit '{}' target '{}' cannot include target-incompatible API '{}'.",
                        unit.name,
                        unit.target.as_str(),
                        api
                    ),
                )
                .context(BuildDiagContext {
                    unit: Some(unit.name.clone()),
                    target: Some(unit.target.as_str().to_string()),
                    edge_kind: Some("NormalImport"),
                    dependency_path: vec![
                        entry_path.display().to_string(),
                        module.display().to_string(),
                        api,
                    ],
                    ..BuildDiagContext::default()
                }));
            }
        }
    }
    Ok(())
}
