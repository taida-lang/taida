/// Cross-backend pins for the list-performance track's correctness
/// findings.
mod common;

use common::{run_interpreter, taida_bin, unique_temp_dir, wasmtime_bin};
use std::path::Path;
use std::process::Command;

fn build_and_run_native(td: &Path, dir: &Path, stem: &str) -> String {
    let bin = dir.join(format!("{stem}_native"));
    let status = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("taida build native runs");
    assert!(status.success(), "native build failed for {stem}");
    let out = Command::new(&bin).output().expect("native binary runs");
    assert!(out.status.success(), "native run failed for {stem}");
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn build_and_run_wasm(td: &Path, dir: &Path, stem: &str) -> Option<String> {
    let wasmtime = wasmtime_bin()?;
    let wasm = dir.join(format!("{stem}.wasm"));
    let status = Command::new(taida_bin())
        .args(["build", "wasm-min"])
        .arg(td)
        .arg("-o")
        .arg(&wasm)
        .status()
        .expect("taida build wasm-min runs");
    assert!(status.success(), "wasm build failed for {stem}");
    let out = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("wasmtime runs");
    assert!(out.status.success(), "wasm run failed for {stem}");
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn assert_parity(dir: &Path, stem: &str, source: &str) -> String {
    let td = dir.join(format!("{stem}.td"));
    std::fs::write(&td, source).expect("write fixture");
    let interp = run_interpreter(&td).expect("interpreter runs");
    let native = build_and_run_native(&td, dir, stem);
    assert_eq!(interp, native, "{stem}: interp vs native");
    if let Some(wasm) = build_and_run_wasm(&td, dir, stem) {
        assert_eq!(interp, wasm, "{stem}: interp vs wasm-min");
    } else {
        eprintln!("SKIP: wasmtime not found, wasm leg skipped for {stem}");
    }
    interp
}

/// A function whose body ends in an unmolded Fold could not be written
/// at all: the static return-type table read the LIST argument instead
/// of the INIT accumulator, so `=> :Int` failed with "body returns
/// @[Int]" on every such function.
#[test]
fn function_ending_in_fold_typechecks_and_runs() {
    let dir = unique_temp_dir("f60_fold_ret");
    let out = assert_parity(
        &dir,
        "fold_ret",
        r#"sumIt u: Int =
  xs <= @[1, 2, 3]
  Fold[xs, 0, _ acc: Int x: Int = acc + x]() >=> t
  t
=> :Int

joinIt u: Int =
  ws <= @["a", "b"]
  Foldr[ws, "", _ acc: Str w: Str = acc + w]() >=> s
  s.length()
=> :Int

stdout(sumIt(1))
stdout(joinIt(1))
"#,
    );
    assert_eq!(out, "6\n2");
    let _ = std::fs::remove_dir_all(&dir);
}
