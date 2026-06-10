/// Regression tests for the polymorphic string-misclassification family.
///
/// The native runtime used to identify "string values" by the heuristic
/// "mapped page without a container magic at v[0] -> raw char*". Any Int
/// that crossed into a mapped address range (the no-pie ELF load base is
/// 0x400000 = 4,194,304, so an accumulator only needs to reach ~4.2M)
/// was silently reclassified as a string: polymorphic `+` turned into
/// string concatenation, display printed text from a fabricated pointer,
/// `==` ran strcmp on it, and Set/HashMap hashing followed the same path.
/// Deep tail recursion with an `>=>` unmold in the loop body was the
/// first reproduction (the unmold result was untyped, so `acc + val`
/// lowered to the polymorphic add and the accumulator crossed the ELF
/// base after a few hundred iterations).
///
/// The fix is two-sided:
/// - lowering: `Lax[x]() >=> v` / all-Int `Div`/`Mod` unmolds land their
///   statically-known kind in the typed sets, keeping the arithmetic on
///   the `taida_int_*` path;
/// - runtime: `taida_is_string_value` requires the hidden-header magic
///   (heap / static / rope), and every display / typeof / hash / JSON
///   judgment site shares that positive identification.
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

/// Run one fixture across interp / native / wasm-min and assert identical
/// stdout. Returns the agreed output for additional assertions.
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

/// Core reproduction: one unmold inside a deep tail-recursive cond-arm
/// body. The accumulator crosses the ELF load base (~4.19M) around
/// depth 9571, where the broken heuristic started concatenating.
#[test]
fn deep_tail_recursion_with_unmold_keeps_int_semantics() {
    let dir = unique_temp_dir("f58b003_deep_unmold");
    let out = assert_parity(
        &dir,
        "deep_unmold",
        r#"loopA n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    Lax[n]() >=> val
    loopA(n - 1, acc + val)
=> :Int
stdout(loopA(10000, 0))
"#,
    );
    assert_eq!(out, "50005000");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two chained unmolds (the bench_mold_unmold shape): Lax + all-Int Div.
#[test]
fn chained_div_unmold_parity() {
    let dir = unique_temp_dir("f58b003_div_unmold");
    let out = assert_parity(
        &dir,
        "div_unmold",
        r#"laxLoop n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    Lax[n]() >=> val
    Div[val, 3]() >=> divided
    laxLoop(n - 1, acc + divided)
=> :Int
stdout(laxLoop(10000, 0))
"#,
    );
    assert_eq!(out, "16665000");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Untyped params force the polymorphic runtime path; Ints above the
/// ELF load base / heap range must stay Ints for `+` and display.
/// (`a == b` on untyped params is rejected by the checker with a
/// type-annotation diagnostic, so the equality leg lives in
/// `large_int_set_membership_parity` via the Set equality engine.)
#[test]
fn large_int_on_polymorphic_path_stays_int() {
    let dir = unique_temp_dir("f58b003_poly_int");
    let out = assert_parity(
        &dir,
        "poly_int",
        r#"addUk a b = a + b => :Int
stdout(addUk(4200000, 5000))
stdout(addUk(8000000, 1))
"#,
    );
    assert_eq!(out, "4205000\n8000001");
    let _ = std::fs::remove_dir_all(&dir);
}

/// String behaviour must survive the positive-identification rewrite:
/// concatenation, Lax string defaults, and typeof.
#[test]
fn string_paths_still_recognised() {
    let dir = unique_temp_dir("f58b003_str_regress");
    let out = assert_parity(
        &dir,
        "str_regress",
        r#"s <= "con" + "cat"
stdout(s)
Lax["fallback"]() >=> d
stdout(d)
joinUk a b = a + b => :Str
stdout(joinUk("x", "y"))
"#,
    );
    assert_eq!(out, "concat\nfallback\nxy");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Set/HashMap hashing shares the same string judgment: a large Int key
/// must hash as an Int (not as bytes read from a fabricated pointer).
#[test]
fn large_int_set_membership_parity() {
    let dir = unique_temp_dir("f58b003_set_hash");
    let out = assert_parity(
        &dir,
        "set_hash",
        r#"s <= setOf(@[4200000, 8400000, 4200000])
stdout(s.size())
stdout(s.has(4200000))
stdout(s.has(4200001))
"#,
    );
    assert_eq!(out, "2\ntrue\nfalse");
    let _ = std::fs::remove_dir_all(&dir);
}
