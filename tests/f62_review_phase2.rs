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

/// F62B-031: a single-line cond arm body supports pipeline and unmold
/// continuations — the dangling `=>` used to hit the `=> :Type` recovery
/// placeholder and break the whole cond parse.
#[test]
fn single_line_arm_body_supports_pipe_continuations() {
    let bind = run_interp(
        "f62rp2_arm_pipe_bind",
        concat!(
            "check x: Int =\n",
            "  | x > 0 |> stdout(\"pos\") => y\n",
            "  | _ |> stdout(\"neg\")\n",
            "=> :Int\n",
            "check(1)\n",
        ),
    );
    assert!(bind.status.success(), "stderr={}", stderr_text(&bind));
    assert_eq!(stdout_text(&bind), "pos\n");

    let unmold = run_interp(
        "f62rp2_arm_unmold",
        concat!(
            "pick x: Int =\n",
            "  | x > 0 |> Div[10, 2]() >=> v\n",
            "  | _ |> 0\n",
            "=> :Int\n",
            "stdout(pick(1).toString())\n",
        ),
    );
    assert!(unmold.status.success(), "stderr={}", stderr_text(&unmold));
    assert_eq!(stdout_text(&unmold), "5\n");

    // The trailing bare identifier is the tail binding (same rule as the
    // statement-level pipeline): `x => double => r` binds 6 to r and the
    // arm yields it.
    let stages = run_interp(
        "f62rp2_arm_stages",
        concat!(
            "double n: Int = n * 2 => :Int\n",
            "pick x: Int =\n",
            "  | x > 0 |> x => double => r\n",
            "  | _ |> 0\n",
            "=> :Int\n",
            "stdout(pick(3).toString())\n",
        ),
    );
    assert!(stages.status.success(), "stderr={}", stderr_text(&stages));
    assert_eq!(stdout_text(&stages), "6\n");
}

/// F62B-034: the custom mold unmold hook runs on native (it used to fall
/// back to the filling/__value channel — 7 instead of 70).
#[test]
fn custom_mold_unmold_hook_runs_on_native() {
    let dir = unique_temp_dir("f62rp2_native_hook");
    let src = dir.join("main.td");
    write_file(
        &src,
        concat!(
            "Mold[T] => Tenfold[T] = @(\n",
            "  unmold _ = filling * 10 => :T\n",
            ")\n",
            "w <= Tenfold[7]()\n",
            "w >=> x\n",
            "stdout(x.toString())\n",
        ),
    );
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("native build");
    assert!(
        build.status.success(),
        "native build must succeed\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&bin).output().expect("run native");
    let _ = fs::remove_dir_all(&dir);
    assert!(run.status.success());
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "70\n",
        "native must run the unmold hook"
    );
}

/// Final-review #1: a schema-passing generic cannot be used as a value
/// (binding / argument / pipeline) — the hidden schema parameters only
/// flow through explicit-type-argument call sites.
#[test]
fn schema_passing_generic_cannot_be_a_value() {
    let output = run_interp(
        "f62rp2_schema_value_ref",
        concat!(
            "queryAll[T] db: CageBuilder  sql: Str =\n",
            "  db => InCage[_, \"q\", @[sql]]() => Uncage[_, \"all\", T]() >=> rows\n",
            "  rows\n",
            "=> :T\n",
            "g <= queryAll\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1510]")
            && stderr_text(&output).contains("cannot be used as a value"),
        "expected the value-reference rejection, got: {}",
        stderr_text(&output)
    );
}

/// Final-review #3: composite Out types (`@[T]`) have no hidden-schema
/// representation and are rejected at definition with the workaround named.
#[test]
fn composite_out_type_param_rejected_at_definition() {
    let output = run_interp(
        "f62rp2_composite_out",
        concat!(
            "queryAll[T] db: CageBuilder  sql: Str =\n",
            "  db => InCage[_, \"q\", @[sql]]() => Uncage[_, \"all\", @[T]]() >=> rows\n",
            "  rows\n",
            "=> :@[T]\n",
            "stdout(\"checked\")\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1510]")
            && stderr_text(&output).contains("composite host-call Out"),
        "expected the composite-Out rejection, got: {}",
        stderr_text(&output)
    );
}

/// Final-review #5 (decided as a fixed constraint): explicit-type-argument
/// calls take exactly the declared parameter count — defaults are not
/// omittable in the explicit form.
#[test]
fn explicit_generic_call_requires_exact_value_arity() {
    let output = run_interp(
        "f62rp2_exact_arity",
        concat!(
            "pad[T] x: T  suffix: Str <= \"!\" =\n",
            "  x\n",
            "=> :T\n",
            "y <= pad[Int](5)\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1301]"),
        "expected the exact-arity rejection, got: {}",
        stderr_text(&output)
    );
}

/// Final-review #11: scalar (non-list) InCage args are rejected.
#[test]
fn incage_scalar_args_rejected() {
    let output = run_interp(
        "f62rp2_incage_scalar",
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "Cage[db]() => InCage[_, \"get\", \"key\"]()\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E3601]"),
        "expected the wire-list rejection, got: {}",
        stderr_text(&output)
    );
}

/// Final-review #4: the host-capability surface rejects at JS build time
/// instead of emitting undefined runtime symbols.
#[test]
fn js_backend_rejects_host_capability_surface() {
    let dir = unique_temp_dir("f62rp2_js_reject");
    let src = dir.join("main.td");
    write_file(
        &src,
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "Cage[db]() => InCage[_, \"q\", @[]]() => Uncage[_, \"all\", Str]() >=> rows\n",
            "stdout(rows)\n",
        ),
    );
    let mjs = dir.join("main.mjs");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg("js")
        .arg(&src)
        .arg("-o")
        .arg(&mjs)
        .output()
        .expect("js build");
    let _ = fs::remove_dir_all(&dir);
    assert!(!build.status.success(), "js build must reject");
    assert!(
        String::from_utf8_lossy(&build.stderr).contains("not available on the JS backend"),
        "expected the JS support-matrix rejection, got: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}
