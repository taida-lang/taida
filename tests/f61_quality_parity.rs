/// Cross-backend pins for the quality-hardening track: checker type
/// hints for HOF-mold callbacks, and the Min/Max mold family.
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

/// Unannotated lambdas in HOF-mold positions infer their parameter
/// types from the list (the same expected-type hint method-position
/// lambdas get); Map's checker return type follows the callback (a
/// type-changing map used to read back as the INPUT list type and fail
/// downstream); Find returns Lax of the element type (it was a
/// hardcoded Lax[Unknown] that made every bare `>=>` unresolvable).
#[test]
fn mold_callbacks_infer_without_annotations() {
    let dir = unique_temp_dir("f61_mold_hints");
    let out = assert_parity(
        &dir,
        "hints",
        r#"nums <= @[1, 2, 3]
Map[nums, _ x = x * 2]() >=> doubled
stdout(doubled)
Map[nums, _ x = x.toString()]() >=> strs
stdout(strs)
Filter[doubled, _ x = x > 2]() >=> big
stdout(big)
Find[@[1, 5, 3], _ x = x > 2]() >=> found
stdout(found)
Fold[nums, 0, _ acc x = acc + x]() >=> total
stdout(total)
useStrs xs: @[Int] = Map[xs, _ x = x.toString()]() => :@[Str]
stdout(useStrs(nums))
stdout(Count[nums, _ x = x > 1]())
"#,
    );
    assert_eq!(
        out,
        "@[2, 4, 6]\n@[\"1\", \"2\", \"3\"]\n@[4, 6]\n5\n6\n@[\"1\", \"2\", \"3\"]\n2"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A lambda with no context to take a type from must still be
/// rejected: annotations are "write them when the context cannot
/// supply the type", not never.
#[test]
fn contextless_ambiguous_lambda_still_rejected() {
    let dir = unique_temp_dir("f61_lambda_neg");
    let td = dir.join("ambiguous.td");
    std::fs::write(&td, "f <= _ x y = x + y\nstdout(\"no\")\n").expect("write fixture");
    let out = Command::new(taida_bin())
        .arg(&td)
        .output()
        .expect("taida runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("[E1527]"),
        "ambiguous lambda must keep E1527, got: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Min[]/Max[] molds: registered in the spec table but unimplemented on
/// every backend (the interpreter leaked the raw MoldInst pack, JS
/// crashed on ReferenceError, native/wasm failed to lower). Pins the
/// mold form, the kind-aware ordering (Float / Str), the empty-list
/// Lax, and the method twins' element-kind display.
#[test]
fn min_max_molds_work_on_all_backends() {
    let dir = unique_temp_dir("f61_min_max");
    let out = assert_parity(
        &dir,
        "min_max",
        r#"mn <= Min[@[3, 1, 2]]()
stdout(mn.getOrDefault(0))
mx <= Max[@[3, 1, 2]]()
stdout(mx.getOrDefault(0))
Min[@[1.5, 2.5, 0.5]]() >=> fmin
stdout(fmin)
Max[@[-0.5, -1.5]]() >=> fmax
stdout(fmax)
Min[@["b", "a", "c"]]() >=> smin
stdout(smin)
e: Lax[Int] <= Min[@[]]()
stdout(e.hasValue())
fl <= @[2.5, 0.5]
fl.min() >=> mmin
stdout(mmin)
mixed <= @[3, 1, 2]
stdout(Max[mixed]().getOrDefault(0))
"#,
    );
    assert_eq!(out, "1\n3\n0.5\n-0.5\na\nfalse\n0.5\n3");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Float kind through read paths: pack-field reads, getOrDefault with
/// a Float default, If/Abs mold results — each used to display as raw
/// f64 bits on the compiled backends (the value heuristics cannot
/// identify a Float payload; the static classifiers now carry it).
#[test]
fn float_kind_flows_through_read_paths() {
    let dir = unique_temp_dir("f61_float_reads");
    let out = assert_parity(
        &dir,
        "float_reads",
        r#"p <= @(x <= 1.5)
stdout(p.x.toString())
m <= hashMap().set("a", 0.5)
m.get("a").getOrDefault(0.0) >=> g
stdout(g)
If[true, 1.5, 2.5]() >=> iv
stdout(iv)
Abs[-1.5]() >=> av
stdout(av)
"#,
    );
    assert_eq!(out, "1.5\n0.5\n1.5\n1.5");
    let _ = std::fs::remove_dir_all(&dir);
}

/// jsonEncode renders Float fields as numbers, not their raw f64 bit
/// patterns ({"score":4611686018427387904} corrupted every serialised
/// artifact downstream). The per-slot pack tags carry the kind; the
/// serialiser reads them for plain packs too, and the Float hint
/// formats through the shared Rust-Display-compatible path.
#[test]
fn json_encode_renders_float_fields() {
    let dir = unique_temp_dir("f61_json_float");
    let out = assert_parity(
        &dir,
        "json_float",
        r#"whole <= @(score <= 2.0, ok <= false, n <= 7, s <= "hi")
stdout(jsonEncode(whole))
nested <= @(inner <= @(rate <= -0.5))
stdout(jsonEncode(nested))
"#,
    );
    assert_eq!(
        out,
        "{\"n\":7,\"ok\":false,\"s\":\"hi\",\"score\":2.0}\n{\"inner\":{\"rate\":-0.5}}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
