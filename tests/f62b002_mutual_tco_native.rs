// F62B-002: tail-only mutual-recursion cycles compile and run on the
// C-lowering backends (native / wasm) via the dispatcher merge — guide 09's
// promise ("mutual tail recursion is optimized") holds on every backend.
//
// The lowering merges each mergeable cycle into one self-tail-recursive
// dispatcher (tag + per-member slots) plus thin wrappers, so the existing
// self-TCO loop machinery applies. Cycles outside the mergeable subset
// (non-tail calls, partial arity, nested intra-cycle args) keep the
// [E0700] reject.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn write_td(label: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = unique_temp_dir(label);
    let td = dir.join("main.td");
    write_file(&td, source);
    (dir, td)
}

fn run_interp(td: &Path) -> Output {
    Command::new(taida_bin())
        .arg(td)
        .output()
        .expect("run interpreter")
}

fn build_and_run_native(dir: &Path, td: &Path) -> Result<Output, String> {
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run native build");
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).into_owned());
    }
    Ok(Command::new(&bin).output().expect("run native binary"))
}

const DEEP_MUTUAL: &str = r#"pingA n: Int =
  | n < 1 |> "done"
  | _ |> pingB(n - 1)
=> :Str

pingB n: Int =
  | n < 1 |> "done"
  | _ |> pingA(n - 1)
=> :Str

stdout(pingA(100000))
"#;

/// The guide 09 shape: a two-member tail cycle at depth 100k must compile
/// natively, not overflow, and match the interpreter.
#[test]
fn native_runs_deep_tail_mutual_recursion() {
    let (dir, td) = write_td("f62b002_deep", DEEP_MUTUAL);
    let interp = run_interp(&td);
    assert!(interp.status.success(), "interp must run");
    let native = build_and_run_native(&dir, &td).expect("native must compile");
    assert!(
        native.status.success(),
        "native must run\nstderr={}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        String::from_utf8_lossy(&native.stdout),
        "interp / native parity"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Three-member cycle with mixed arities and swapped parameter orders,
/// entered both directly and through a non-cycle caller — wrappers must
/// keep every external entry point working.
#[test]
fn native_runs_three_member_mixed_arity_cycle() {
    let source = r#"stepA n: Int  acc: Str =
  | n < 1 |> acc
  | _ |> stepB(n - 1, `${acc}a`)
=> :Str

stepB n: Int  acc: Str =
  | n < 1 |> acc
  | _ |> stepC(acc, n - 1)
=> :Str

stepC acc: Str  n: Int =
  | n < 1 |> acc
  | _ |> stepA(n - 1, `${acc}c`)
=> :Str

kick n: Int =
  stepA(n, "k")
=> :Str

stdout(stepA(7, ""))
stdout(stepB(2, "x"))
stdout(kick(4))
"#;
    let (dir, td) = write_td("f62b002_three", source);
    let interp = run_interp(&td);
    assert!(interp.status.success(), "interp must run");
    let native = build_and_run_native(&dir, &td).expect("native must compile");
    assert!(native.status.success(), "native must run");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout),
        String::from_utf8_lossy(&native.stdout),
        "interp / native parity"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Non-tail mutual cycles stay rejected at compile time ([E1614] from the
/// cross-backend check, [E0700] from the native check).
#[test]
fn native_still_rejects_non_tail_mutual_cycle() {
    let source = r#"oddSum n: Int =
  | n < 1 |> 0
  | _ |> 1 + evenSum(n - 1)
=> :Int

evenSum n: Int =
  | n < 1 |> 0
  | _ |> 1 + oddSum(n - 1)
=> :Int

stdout(oddSum(10).toString())
"#;
    let (dir, td) = write_td("f62b002_nontail", source);
    let err = build_and_run_native(&dir, &td).expect_err("non-tail cycle must be rejected");
    assert!(err.contains("[E0700]"), "expected [E0700] in: {err}");
    let _ = fs::remove_dir_all(&dir);
}

/// wasm-wasi shares the lowering: the deep cycle must run there too when
/// wasmtime is available.
#[test]
fn wasm_wasi_runs_deep_tail_mutual_recursion() {
    let wasmtime = match common::wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: wasmtime unavailable");
            return;
        }
    };
    let (dir, td) = write_td("f62b002_wasm", DEEP_MUTUAL);
    let wasm = dir.join("main.wasm");
    let build = Command::new(taida_bin())
        .args(["build", "wasm-wasi"])
        .arg(&td)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("wasm build");
    assert!(
        build.status.success(),
        "wasm-wasi must compile\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&wasmtime)
        .args(["run", "--"])
        .arg(&wasm)
        .output()
        .expect("wasmtime run");
    assert!(run.status.success(), "wasm must run");
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("done"),
        "expected 'done', got: {}",
        String::from_utf8_lossy(&run.stdout)
    );
    let _ = fs::remove_dir_all(&dir);
}
