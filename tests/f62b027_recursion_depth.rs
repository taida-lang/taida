// F62B-027: non-tail recursion depth overruns fail diagnostically on every
// backend instead of crashing, and the interpreter limit is deep enough for
// real tree walks.
//
// - interpreter: MAX_CALL_DEPTH raised 256 -> 8192, evaluation runs on a
//   dedicated large-stack thread so the raised limit actually fits.
// - native: a stack-watermark guard at every user-function entry (outside
//   the TCO loop) exits with a diagnostic instead of the former silent
//   SIGSEGV. Typical native capacity far exceeds the interpreter's 8192,
//   so the interpreter stays the binding constraint for parity.
// - wasm: wasmtime already traps with "call stack exhausted" (unchanged).

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::process::Command;

fn deep_program(n: u64) -> String {
    format!(
        "deep n: Int =\n  | n < 1 |> 0\n  | _ |> 1 + deep(n - 1)\n=> :Int\n\nstdout(deep({n}).toString())\n"
    )
}

#[test]
fn interpreter_handles_depth_8000() {
    let dir = unique_temp_dir("f62b027_interp_ok");
    let td = dir.join("main.td");
    write_file(&td, &deep_program(8000));
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(
        out.status.success(),
        "depth 8000 must run\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("8000"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interpreter_rejects_depth_overrun_diagnostically() {
    let dir = unique_temp_dir("f62b027_interp_over");
    let td = dir.join("main.td");
    write_file(&td, &deep_program(9000));
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(!out.status.success(), "depth 9000 must be rejected");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Maximum call depth (8192) exceeded"),
        "expected the depth diagnostic, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_runaway_recursion_exits_diagnostically_not_sigsegv() {
    let dir = unique_temp_dir("f62b027_native");
    let td = dir.join("main.td");
    write_file(&td, &deep_program(10_000_000));
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(&td)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("build");
    assert!(build.status.success());
    let out = Command::new(&bin).output().expect("run binary");
    assert_eq!(
        out.status.code(),
        Some(1),
        "native must exit 1 (not die on a signal); status: {:?}",
        out.status
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Maximum call stack depth exceeded"),
        "expected the stack-guard diagnostic, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_depth_8000_still_runs() {
    let dir = unique_temp_dir("f62b027_native_ok");
    let td = dir.join("main.td");
    write_file(&td, &deep_program(8000));
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(&td)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("build");
    assert!(build.status.success());
    let out = Command::new(&bin).output().expect("run binary");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("8000"));
    let _ = std::fs::remove_dir_all(&dir);
}
