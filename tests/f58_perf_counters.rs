/// Integration tests for the TAIDA_PERF_COUNTERS measurement build.
///
/// The allocation-path counters are a development-only measurement surface:
/// setting `TAIDA_PERF_COUNTERS=1` in the environment of `taida build`
/// compiles the runtime with `-DTAIDA_PERF_COUNTERS`, which arms the
/// counters and an exit dump on stderr. The normal build must stay
/// byte-for-byte free of the hooks (no dump line, unchanged stdout).
///
/// These tests pass the variable via `Command::env` so the gate is scoped
/// to the child build process and never leaks into parallel tests.
mod common;

use common::{taida_bin, unique_temp_dir, wasmtime_bin};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixture that exercises every native allocation tier: a list literal
/// (arena / malloc), string concatenation (str freelist candidates), and
/// scalar arithmetic (must stay allocation-free).
const FIXTURE: &str = r#"xs <= @[1, 2, 3, 4, 5]
label <= "len=" + xs.length().toString()
stdout(label)
"#;

fn write_fixture(dir: &Path) -> PathBuf {
    let td = dir.join("perf_fixture.td");
    std::fs::write(&td, FIXTURE).expect("write fixture");
    td
}

fn build(td: &Path, out: &Path, target: &str, perf: bool) {
    let mut cmd = Command::new(taida_bin());
    cmd.arg("build").arg(target).arg(td).arg("-o").arg(out);
    if perf {
        cmd.env("TAIDA_PERF_COUNTERS", "1");
    } else {
        cmd.env_remove("TAIDA_PERF_COUNTERS");
    }
    let status = cmd.status().expect("taida build runs");
    assert!(status.success(), "taida build {} failed", target);
}

/// Parse `key=value` integers out of a `TAIDA_PERF ...` dump line.
fn parse_dump(line: &str) -> Vec<(String, u64)> {
    line.split_whitespace()
        .filter_map(|tok| {
            let (k, v) = tok.split_once('=')?;
            Some((k.to_string(), v.parse::<u64>().ok()?))
        })
        .collect()
}

#[test]
fn native_normal_build_emits_no_perf_dump() {
    let dir = unique_temp_dir("f58_perf_native_off");
    let td = write_fixture(&dir);
    let bin = dir.join("fixture_bin");
    build(&td, &bin, "native", false);

    let out = Command::new(&bin).output().expect("fixture runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "len=5");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("TAIDA_PERF"),
        "normal build must not emit a perf dump, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_perf_build_emits_parsable_dump() {
    let dir = unique_temp_dir("f58_perf_native_on");
    let td = write_fixture(&dir);
    let bin = dir.join("fixture_bin");
    build(&td, &bin, "native", true);

    let out = Command::new(&bin).output().expect("fixture runs");
    assert!(out.status.success());
    // The measurement build must not change observable program output.
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "len=5");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let dump = stderr
        .lines()
        .find(|l| l.starts_with("TAIDA_PERF native "))
        .unwrap_or_else(|| panic!("perf build must emit a native dump line, got: {stderr}"));
    let kv = parse_dump(dump);
    let get = |k: &str| {
        kv.iter()
            .find(|(key, _)| key == k)
            .unwrap_or_else(|| panic!("dump line missing {k}: {dump}"))
            .1
    };
    // The fixture allocates a 5-element list + strings, so the allocator
    // must have been exercised through arena and/or malloc.
    assert!(
        get("arena_calls") + get("malloc_calls") > 0,
        "fixture allocations must be visible in the counters: {dump}"
    );
    assert!(
        get("arena_bytes") + get("malloc_bytes") > 0,
        "allocated bytes must be visible in the counters: {dump}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wasm_min_normal_build_emits_no_perf_dump() {
    let Some(wasmtime) = wasmtime_bin() else {
        eprintln!("SKIP: wasmtime not found, skipping wasm perf-counter test");
        return;
    };
    let dir = unique_temp_dir("f58_perf_wasm_off");
    let td = write_fixture(&dir);
    let wasm = dir.join("fixture.wasm");
    build(&td, &wasm, "wasm-min", false);

    let out = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("wasmtime runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "len=5");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("TAIDA_PERF"),
        "normal wasm build must not emit a perf dump, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wasm_min_perf_build_emits_parsable_dump() {
    let Some(wasmtime) = wasmtime_bin() else {
        eprintln!("SKIP: wasmtime not found, skipping wasm perf-counter test");
        return;
    };
    let dir = unique_temp_dir("f58_perf_wasm_on");
    let td = write_fixture(&dir);
    let wasm = dir.join("fixture.wasm");
    build(&td, &wasm, "wasm-min", true);

    let out = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("wasmtime runs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "len=5");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let dump = stderr
        .lines()
        .find(|l| l.starts_with("TAIDA_PERF wasm "))
        .unwrap_or_else(|| panic!("perf build must emit a wasm dump line, got: {stderr}"));
    let kv = parse_dump(dump);
    let alloc_calls = kv
        .iter()
        .find(|(k, _)| k == "alloc_calls")
        .expect("dump has alloc_calls")
        .1;
    assert!(
        alloc_calls > 0,
        "fixture allocations must be visible in the wasm counters: {dump}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
