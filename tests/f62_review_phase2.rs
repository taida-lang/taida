// Phase-2 boundary second-opinion follow-ups (Codex review, all verified):
//
//   High-1   Lte/Between obey the comparison operand rule ([E1605]).
//   High-2   E1545 covers HashMap/Set/Function sources and the
//            `.unmold()` method spelling.
//   M-1      a block-bodied lambda's tail unmold binding types as the
//            BOUND value, not the source.
//   M-2      the `__value` channel demands a `__type` tag on every
//            backend (a plain pack with `__value` is still a plain pack).
//   M-3      alias↔function name collisions are [E1501] in BOTH orders.
//   M-4      alias forward references resolve (fixpoint registration).
//   M-5      (spec'd limit) a bare mold in a value position INSIDE another
//            mold's bracket arguments stays accepted.
//   Low-1    the non-Ident-source `>=> name: T <=<` form reports [E0302].

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::fs;
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

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// High-1: cross-Enum operands are rejected in the mold forms exactly
/// like the operator forms.
#[test]
fn lte_between_reject_cross_enum_operands() {
    let lte = run_interp(
        "f62rp2_lte_enum",
        "Enum => A = :X :Y\nEnum => B = :X :Y\nstdout(Lte[A:X(), B:Y()]().toString())\n",
    );
    assert!(!lte.status.success());
    assert!(
        stderr_text(&lte).contains("[E1605]") && stderr_text(&lte).contains("Lte"),
        "expected E1605 for cross-Enum Lte, got: {}",
        stderr_text(&lte)
    );

    let between = run_interp(
        "f62rp2_between_enum",
        "Enum => A = :X :Y\nstdout(Between[A:X(), 1, 10]().toString())\n",
    );
    assert!(!between.status.success());
    assert!(
        stderr_text(&between).contains("[E1605]"),
        "expected E1605 for Enum-vs-Int Between, got: {}",
        stderr_text(&between)
    );

    // Valid pairs keep working.
    let ok = run_interp(
        "f62rp2_lte_ok",
        "stdout(Lte[1, 2]().toString())\nstdout(Between[\"b\", \"a\", \"c\"]().toString())\n",
    );
    assert!(ok.status.success(), "stderr={}", stderr_text(&ok));
    assert_eq!(stdout_text(&ok), "true\ntrue\n");
}

/// High-2: container and function sources are statically bare, and the
/// `.unmold()` method spelling takes the same rule.
#[test]
fn e1545_covers_containers_functions_and_method_form() {
    let hm = run_interp(
        "f62rp2_hashmap",
        "m: HashMap[Int, Int] <= hashMap().set(1, 2)\nm >=> leaked\nstdout(\"ok\")\n",
    );
    assert!(!hm.status.success());
    assert!(
        stderr_text(&hm).contains("[E1545]"),
        "expected E1545 for HashMap source, got: {}",
        stderr_text(&hm)
    );

    let f = run_interp(
        "f62rp2_fn_unmold",
        "f <= _ x: Int = x + 1\nbad <= f.unmold()\n",
    );
    assert!(!f.status.success());
    assert!(
        stderr_text(&f).contains("[E1545]"),
        "expected E1545 for function .unmold(), got: {}",
        stderr_text(&f)
    );

    // The legal method spelling is untouched.
    let lax = run_interp(
        "f62rp2_lax_unmold",
        "v <= Lax[5]().unmold()\nstdout(v.toString())\n",
    );
    assert!(lax.status.success(), "stderr={}", stderr_text(&lax));
    assert_eq!(stdout_text(&lax), "5\n");
}

/// M-1: the tail unmold binding yields the bound value's type — the
/// formerly-rejected correct annotation checks, the formerly-accepted
/// wrong annotation is rejected.
#[test]
fn block_lambda_tail_unmold_types_as_bound_value() {
    let ok = run_interp(
        "f62rp2_block_ok",
        "g <= _ n: Int =\n  Lax[n]() >=> x\ny: Int <= g(7)\nstdout(y.toString())\n",
    );
    assert!(ok.status.success(), "stderr={}", stderr_text(&ok));
    assert_eq!(stdout_text(&ok), "7\n");

    let bad = run_interp(
        "f62rp2_block_bad",
        "g <= _ n: Int =\n  Lax[n]() >=> x\nz: Lax[Int] <= g(7)\n",
    );
    assert!(
        !bad.status.success(),
        "Lax[Int] annotation over the unmolded Int must now be rejected"
    );
    assert!(
        stderr_text(&bad).contains("Type mismatch"),
        "expected annotation mismatch, got: {}",
        stderr_text(&bad)
    );
}

/// M-2: a plain pack with a `__value` field (and no `__type`) is gorilla
/// on the interpreter too — unified with native/wasm.
#[test]
fn plain_pack_with_value_channel_is_gorilla() {
    let output = run_interp_no_check(
        "f62rp2_value_channel",
        "p <= @(__value <= 5)\np >=> v\nstdout(v.toString())\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1545]") && stderr.contains("><"),
        "expected the plain-pack gorilla, got: {stderr}"
    );
}

/// M-3: alias-then-function and function-then-alias both collide.
#[test]
fn alias_function_collision_detected_both_orders() {
    for (label, src) in [
        (
            "f62rp2_alias_first",
            "Pairs = @[Int]\nPairs x: Int =\n  x\n=> :Int\n",
        ),
        (
            "f62rp2_fn_first",
            "Pairs x: Int =\n  x\n=> :Int\nPairs = @[Int]\n",
        ),
    ] {
        let output = run_interp(label, src);
        assert!(!output.status.success(), "{label} must be rejected");
        assert!(
            stderr_text(&output).contains("[E1501]"),
            "{label}: expected E1501, got: {}",
            stderr_text(&output)
        );
    }
}

/// M-4: forward alias references resolve.
#[test]
fn alias_forward_reference_resolves() {
    let output = run_interp(
        "f62rp2_alias_forward",
        "Grid = @[Row]\nRow = @[Int]\ng: Grid <= @[@[1, 2]]\nstdout(g.length().toString())\n",
    );
    assert!(output.status.success(), "stderr={}", stderr_text(&output));
    assert_eq!(stdout_text(&output), "1\n");
}

/// Low-1: the non-Ident-source typed forward unmold reports [E0302] for a
/// trailing `<=<` instead of a generic parse error.
#[test]
fn typed_forward_unmold_reports_e0302_for_mixed_direction() {
    let output = run_interp("f62rp2_e0302", "Div[10, 2]() >=> half: Int <=< other\n");
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("E0302"),
        "expected E0302, got: {}",
        stderr_text(&output)
    );
}
