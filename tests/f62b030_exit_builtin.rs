// F62B-030: `exit(code)` is a real prelude builtin on every backend.
//
// It was documented (prelude.md §6.1) and registered in the checker since
// gen-D but never implemented: the interpreter died with
// `Undefined variable: 'exit'` and native segfaulted through the unknown-
// variable indirect-call path. It now terminates the process with the
// given code on interpreter / native / wasm-wasi (proc_exit) / JS
// (process.exit); the wasm-edge handler profile traps (no WASI proc_exit).

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::path::PathBuf;
use std::process::Command;

const PROGRAM: &str = r#"configValid <= false
| configValid |> stdout("serving")
| _ |> exit(2)
stdout("unreachable")
"#;

fn write_td(label: &str) -> (PathBuf, PathBuf) {
    let dir = unique_temp_dir(label);
    let td = dir.join("main.td");
    write_file(&td, PROGRAM);
    (dir, td)
}

#[test]
fn interpreter_exits_with_code() {
    let (dir, td) = write_td("f62b030_interp");
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert_eq!(out.status.code(), Some(2), "exit code must be 2");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("unreachable"),
        "control must not pass exit()"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn interpreter_flushes_buffered_output_before_exit() {
    let dir = unique_temp_dir("f62b030_flush");
    let td = dir.join("main.td");
    write_file(&td, "stdout(\"before\")\nexit(0)\n");
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("before"),
        "output written before exit must be flushed, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn native_exits_with_code() {
    let (dir, td) = write_td("f62b030_native");
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(&td)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "native must compile\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&bin).output().expect("run binary");
    assert_eq!(out.status.code(), Some(2), "native exit code must be 2");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("unreachable"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wasm_wasi_exits_with_code() {
    let wasmtime = match common::wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: wasmtime unavailable");
            return;
        }
    };
    let (dir, td) = write_td("f62b030_wasm");
    let wasm = dir.join("main.wasm");
    let build = Command::new(taida_bin())
        .args(["build", "wasm-wasi"])
        .arg(&td)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "wasm-wasi must compile\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&wasmtime)
        .args(["run", "--"])
        .arg(&wasm)
        .output()
        .expect("wasmtime run");
    assert_eq!(out.status.code(), Some(2), "wasm exit code must be 2");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("unreachable"));
    let _ = std::fs::remove_dir_all(&dir);
}
