// F62B-026: unmolding a bare value is rejected instead of passing through.
//
//   Checker [E1545]: a source that is statically a bare value (scalar,
//   list, plain pack type, enum) and not a mold-call form is rejected at
//   compile time. Mold calls (`Mold[...]() >=> x`) are exempt by form —
//   every value mold returns its bare result and that is the documented
//   binding idiom.
//
//   Runtime: a machinery-less plain pack (no `__type` tag, no unmold hook,
//   no `__value` channel) is gorilla (diagnostic + `><` + exit 1) on every
//   backend. Bare scalars / lists stay identity at runtime (the checker is
//   the static line of defence; runtime values carry no mold provenance).
//
// F62B-033 (fixed here): the JS runtime used to return custom mold packs
// unchanged from `>=>` instead of running their unmold hook.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run_interp(label: &str, source: &str) -> Output {
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let output = Command::new(taida_bin())
        .arg(&src)
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    output
}

fn run_interp_no_check(label: &str, source: &str) -> Output {
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let output = Command::new(taida_bin())
        .arg("--no-check")
        .arg(&src)
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    output
}

fn build_and_run(label: &str, source: &str, backend: &str) -> Option<Output> {
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let built: std::path::PathBuf;
    let mut build = Command::new(taida_bin());
    match backend {
        "native" => {
            built = dir.join("main_bin");
            build.arg("build").arg(&src).arg("-o").arg(&built);
        }
        "js" => {
            built = dir.join("main.mjs");
            build.arg("build").arg("js").arg(&src).arg("-o").arg(&built);
        }
        _ => panic!("backend"),
    }
    // The checker rejects these statically; runtime tests compile unchecked.
    build.arg("--no-check");
    let ok = build.output().expect("build").status.success();
    if !ok {
        let _ = fs::remove_dir_all(&dir);
        return None;
    }
    let output = if backend == "js" {
        Command::new("node").arg(&built).output().expect("node")
    } else {
        Command::new(&built).output().expect("run native")
    };
    let _ = fs::remove_dir_all(&dir);
    Some(output)
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ── checker [E1545] ──────────────────────────────────────────────

/// A bare Int literal cannot be unmolded.
#[test]
fn e1545_int_literal_source_rejected() {
    let output = run_interp("f62b026_int", "5 >=> e\n");
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1545]"),
        "expected E1545, got: {}",
        stderr_text(&output)
    );
}

/// A plain pack variable cannot be unmolded — both directions.
#[test]
fn e1545_plain_pack_variable_rejected_both_directions() {
    for (label, src) in [
        ("f62b026_pack_fwd", "p <= @(a <= 1)\np >=> q\n"),
        ("f62b026_pack_bwd", "p <= @(a <= 1)\nq <=< p\n"),
    ] {
        let output = run_interp(label, src);
        assert!(!output.status.success(), "{label} must be rejected");
        assert!(
            stderr_text(&output).contains("[E1545]"),
            "{label}: expected E1545, got: {}",
            stderr_text(&output)
        );
    }
}

/// `.unmold()` on a bare value is rejected at compile time. A list
/// receiver is caught by the method-existence check ([E1509]) before the
/// dedicated [E1545] rule — either way the program never runs.
#[test]
fn unmold_method_on_bare_value_rejected() {
    let output = run_interp("f62b026_method", "x <= @[1, 2]\ny <= x.unmold()\n");
    assert!(!output.status.success());
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1545]") || stderr.contains("[E1509]"),
        "expected a compile-time rejection, got: {stderr}"
    );
}

/// The documented mold-call binding idiom stays legal for every value
/// mold, regardless of its bare result type.
#[test]
fn mold_call_binding_idiom_stays_legal() {
    let output = run_interp(
        "f62b026_idiom",
        concat!(
            "numbers <= @[1, 2, 3]\n",
            "Map[numbers, _ x = x * 2]() >=> doubled\n",
            "stdout(doubled.length().toString())\n",
            "Trim[\"  hi  \"]() >=> t\n",
            "stdout(t)\n",
            "Div[10, 2]() >=> half\n",
            "stdout(half.toString())\n",
        ),
    );
    assert!(
        output.status.success(),
        "mold-call idiom must stay legal\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "3\nhi\n5\n");
}

/// `<=<` direction takes the mold-call exemption identically.
#[test]
fn backward_mold_call_binding_is_exempt() {
    let output = run_interp(
        "f62b026_backward_idiom",
        "t <=< Trim[\"  pad  \"]()\nstdout(t)\n",
    );
    assert!(
        output.status.success(),
        "backward mold-call unmold must stay legal\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "pad\n");
}

// ── runtime gorilla (plain packs, all backends) ──────────────────

const PLAIN_PACK_GORILLA: &str = "p <= @(a <= 1)\np >=> q\nstdout(\"unreachable\")\n";
const GORILLA_DIAG: &str = "[E1545] Cannot unmold a non-mold value";

/// The interpreter gorillas a machinery-less plain pack at runtime
/// (checker off — the static rule is tested above).
#[test]
fn runtime_plain_pack_gorilla_interp() {
    let output = run_interp_no_check("f62b026_rt_interp", PLAIN_PACK_GORILLA);
    assert_eq!(output.status.code(), Some(1));
    assert!(!stdout_text(&output).contains("unreachable"));
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains(GORILLA_DIAG) && stderr.contains("><"),
        "expected gorilla diagnostic, got: {stderr}"
    );
}

/// Native and JS gorilla identically.
#[test]
fn runtime_plain_pack_gorilla_native_and_js() {
    for backend in ["native", "js"] {
        let Some(output) = build_and_run(
            &format!("f62b026_rt_{backend}"),
            PLAIN_PACK_GORILLA,
            backend,
        ) else {
            panic!("{backend} build failed");
        };
        assert_eq!(output.status.code(), Some(1), "{backend} must exit 1");
        assert!(
            !stdout_text(&output).contains("unreachable"),
            "{backend} must terminate before the next statement"
        );
        let stderr = stderr_text(&output);
        assert!(
            stderr.contains(GORILLA_DIAG) && stderr.contains("><"),
            "{backend}: expected gorilla diagnostic, got: {stderr}"
        );
    }
}

/// Bare scalars stay identity at runtime (no mold provenance on values —
/// the checker is the static line of defence). The large values pin the
/// native pointer-heuristic edges (page boundary, >2^32) that previously
/// lived in the crash-regression corpus as bare-operand fixtures.
#[test]
fn runtime_bare_scalar_stays_identity_unchecked() {
    let output = run_interp_no_check(
        "f62b026_rt_scalar",
        "5 >=> e\nstdout(e.toString())\nx1 <= 4097\nx1 >=> y1\nstdout(y1.toString())\nx2 <= 1234567890123\nx2 >=> y2\nstdout(y2.toString())\n",
    );
    assert!(
        output.status.success(),
        "unchecked bare scalar unmold stays identity\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "5\n4097\n1234567890123\n");
}

/// The native pointer heuristic survives raw large-int operands through
/// the unchecked runtime path (the original crash shape: an Int whose
/// value lands in plausible-pointer ranges must not be dereferenced).
#[test]
fn runtime_native_ptr_heuristic_edges_unchecked() {
    let source = "x1 <= 4097\nx1 >=> y1\nstdout(y1.toString())\nx2 <= 1234567890123\nx2 >=> y2\nstdout(y2.toString())\n";
    let Some(output) = build_and_run("f62b026_rt_ptr_edges", source, "native") else {
        panic!("native build failed");
    };
    assert!(
        output.status.success(),
        "native unchecked large-int unmold must not crash\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "4097\n1234567890123\n");
}

// ── F62B-033: custom mold unmold hook runs on JS ─────────────────

const CUSTOM_MOLD: &str = concat!(
    "Mold[T] => Tenfold[T] = @(\n",
    "  unmold _ = filling * 10 => :T\n",
    ")\n",
    "w <= Tenfold[7]()\n",
    "w >=> x\n",
    "stdout(x.toString())\n",
);

/// The custom unmold hook runs through `>=>` on interp and JS.
/// (Native/wasm run it too — the hook compiles into an `__unmold`
/// closure on the instance pack; see the cage-chain-era fix.)
#[test]
fn custom_mold_unmold_hook_runs_interp_and_js() {
    let interp = run_interp("f62b026_cm_interp", CUSTOM_MOLD);
    assert!(interp.status.success(), "stderr={}", stderr_text(&interp));
    assert_eq!(stdout_text(&interp), "70\n");

    let js_dir = unique_temp_dir("f62b026_cm_js");
    let src = js_dir.join("main.td");
    write_file(&src, CUSTOM_MOLD);
    let mjs = js_dir.join("main.mjs");
    let built = Command::new(taida_bin())
        .arg("build")
        .arg("js")
        .arg(&src)
        .arg("-o")
        .arg(&mjs)
        .output()
        .expect("build js");
    assert!(built.status.success(), "js build failed");
    let node = Command::new("node").arg(&mjs).output().expect("node");
    let _ = fs::remove_dir_all(Path::new(&js_dir));
    assert!(node.status.success());
    assert_eq!(
        stdout_text(&node),
        "70\n",
        "JS must run the custom unmold hook (F62B-033)"
    );
}
