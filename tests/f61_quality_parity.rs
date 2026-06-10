/// Cross-backend pins for the quality-hardening track: checker type
/// hints for HOF-mold callbacks, and the Min/Max mold family.
mod common;

use common::{node_available, run_interpreter, taida_bin, unique_temp_dir, wasmtime_bin};
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

/// HashMap/Set displays render uniform Float values as numbers on the
/// compiled backends (native printed raw f64 bits in toString and Set,
/// wasm lost the tag in values()). The interpreter currently prints
/// its internal pack form for these containers — a separate, known
/// reference-side issue — so this pin asserts native/wasm agreement
/// and the absence of bit-pattern leakage rather than interp parity.
#[test]
fn hashmap_set_display_renders_floats_on_compiled_backends() {
    let dir = unique_temp_dir("f61_hm_set_display");
    let td = dir.join("hm.td");
    std::fs::write(
        &td,
        r#"m <= hashMap().set("a", 1.5).set("b", 2.5)
stdout(m.values())
stdout(m)
s <= setOf(@[1.5, 2.5])
stdout(s)
stdout(s.toList())
"#,
    )
    .expect("write fixture");
    let native = build_and_run_native(&td, &dir, "hm");
    assert!(
        !native.contains("4609434218613702656"),
        "native leaked f64 bits: {native}"
    );
    assert!(
        native.contains("HashMap({\"a\": 1.5, \"b\": 2.5})") && native.contains("Set({1.5, 2.5})"),
        "native display shape: {native}"
    );
    if let Some(wasm) = build_and_run_wasm(&td, &dir, "hm") {
        assert_eq!(native, wasm, "native vs wasm display");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Extreme Float magnitudes format identically on interp/native/wasm:
/// wasm printed 1e20 as "0" (the u64 cast is UB past 2^64), native
/// chose %g's scientific notation ("1e+20" / "1e-07") where the
/// reference expands every finite f64 in fixed notation.
#[test]
fn extreme_float_magnitudes_format_consistently() {
    let dir = unique_temp_dir("f61_float_extreme");
    let out = assert_parity(
        &dir,
        "extremes",
        r#"big <= 100000000000000000000.5
stdout(big.toString())
huge <= 123456789012345678901234567890.0
stdout(huge.toString())
tiny <= 0.0000001
stdout(tiny.toString())
neg <= -100000000000000000000.5
stdout(neg.toString())
"#,
    );
    assert_eq!(
        out,
        "100000000000000000000.0\n123456789012345677877719597056.0\n0.0000001\n-100000000000000000000.0"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The interpreter and JS backends now share the native backend's
/// consumable-Append analysis (the tail-recursive accumulator build).
/// Pins the three value-semantics hazards of an in-place push — the
/// caller's binding surviving detach, an alias of that binding, and
/// re-feeding a consume-produced list into the same build — plus the
/// O(n) wall-clock shape: 100k elements on the interpreter completes
/// orders of magnitude inside the old O(n²) time (which needed seconds
/// for 5k).
#[test]
fn append_consume_keeps_value_semantics_on_interp_and_js() {
    let dir = unique_temp_dir("f61_consume");
    let td = dir.join("consume.td");
    std::fs::write(
        &td,
        r#"build acc: @[Int] i: Int =
  | i >= 3 |> acc
  | _ |> build(Append[acc, i](), i + 1)
=> :@[Int]
xs <= @[100]
build(xs, 0) >=> r1
stdout(xs)
stdout(r1)
build(r1, 0) >=> r2
stdout(r1)
stdout(r2)
ys <= xs
build(xs, 1) >=> r3
stdout(ys)
stdout(r3)
"#,
    )
    .expect("write fixture");
    let expected =
        "@[100]\n@[100, 0, 1, 2]\n@[100, 0, 1, 2]\n@[100, 0, 1, 2, 0, 1, 2]\n@[100]\n@[100, 1, 2]";
    let interp = run_interpreter(&td).expect("interpreter runs");
    assert_eq!(interp, expected, "interp consume value semantics");

    if node_available() {
        let mjs = dir.join("consume.mjs");
        let status = Command::new(taida_bin())
            .args(["build", "js"])
            .arg(&td)
            .arg("-o")
            .arg(&mjs)
            .status()
            .expect("taida build js runs");
        assert!(status.success(), "js build failed");
        let out = Command::new("node").arg(&mjs).output().expect("node runs");
        assert!(out.status.success(), "js run failed");
        let js = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        assert_eq!(js, expected, "js consume value semantics");
    } else {
        eprintln!("SKIP: node not found, js leg skipped");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 100k-element sequential build on the interpreter: O(n²) cloning
/// needed ~2s for 5k elements, so 100k would take minutes; the consume
/// path finishes in well under the suite timeout. No wall-clock assert
/// — completing at all is the pin.
#[test]
fn append_consume_interp_100k_completes() {
    let dir = unique_temp_dir("f61_consume_perf");
    let td = dir.join("build100k.td");
    std::fs::write(
        &td,
        r#"build acc: @[Int] i: Int =
  | i >= 100000 |> acc
  | _ |> build(Append[acc, i](), i + 1)
=> :@[Int]
build(@[], 0) >=> result
stdout(result.length())
"#,
    )
    .expect("write fixture");
    let interp = run_interpreter(&td).expect("interpreter runs");
    assert_eq!(interp, "100000");
    let _ = std::fs::remove_dir_all(&dir);
}
