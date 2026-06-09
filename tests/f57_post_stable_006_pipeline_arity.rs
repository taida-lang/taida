// POST-STABLE-006 regression: a placeholder/hole-free pipeline stage call
// injects the piped value as an implicit first argument, so the general
// function arity check ([E1301] too-many) must count it toward the effective
// arity — not only crypto builtins.
//
// Before this fix `data => f(a, b)` (where `f` has arity 2) passed the
// checker because only the *written* arguments (2) were counted, even though
// the lowered call carries 3 values; the mismatch surfaced as a runtime error
// instead of a static [E1301]. These tests pin the static reject (registered
// and generic call paths), the no-false-reject case (written args + injection
// exactly fill the arity), and the plain non-pipeline too-many regression.
//
// Like the other arity suites these drive the interpreter entry point
// (`taida <FILE>`).

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

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_e1301(output: &Output, label: &str) {
    assert!(
        !output.status.success(),
        "{label}: over-arity call must be rejected\nstdout={}\nstderr={}",
        stdout_text(output),
        stderr_text(output)
    );
    assert!(
        stderr_text(output).contains("[E1301]"),
        "{label}: expected [E1301], got: {}",
        stderr_text(output)
    );
}

/// Reject (registered path): a pipeline stage call whose written args plus the
/// injected pipe value exceed the function arity is now a static [E1301] (was
/// a runtime error). `pair` has arity 2; `5 => pair(10, 20)` lowers to
/// `pair(5, 10, 20)`.
#[test]
fn e1301_rejects_pipeline_injection_over_arity() {
    let output = run_interp(
        "f57_006_pipe_over",
        r#"pair a: Int b: Int = a + b => :Int
5 => pair(10, 20)
"#,
    );
    assert_e1301(&output, "pipeline injection over arity");
    assert!(
        stderr_text(&output).contains("piped value counts as the first argument"),
        "expected the pipe-injection note in the diagnostic, got: {}",
        stderr_text(&output)
    );
}

/// Reject (generic path): the same effective-arity rule applies to a generic
/// function call that is a pipeline stage. `genericId` has arity 1;
/// `5 => genericId(10)` lowers to `genericId(5, 10)`.
#[test]
fn e1301_rejects_generic_pipeline_injection_over_arity() {
    let output = run_interp(
        "f57_006_generic_pipe_over",
        r#"genericId[T] x: T = x => :T
5 => genericId(10)
"#,
    );
    assert_e1301(&output, "generic pipeline injection over arity");
}

/// No false reject: when the written arguments plus the injected pipe value
/// exactly fill the arity, the call type-checks and runs. `5 => pair(10)`
/// lowers to `pair(5, 10)` = 15.
#[test]
fn e1301_allows_pipeline_injection_exact_arity() {
    let output = run_interp(
        "f57_006_pipe_exact",
        r#"pair a: Int b: Int = a + b => :Int
5 => pair(10) => stdout(_.toString())
"#,
    );
    let stderr = stderr_text(&output);
    assert!(
        !stderr.contains("[E1301]"),
        "an exact-arity pipeline stage call must not be rejected, got: {stderr}"
    );
    assert!(
        output.status.success(),
        "exact-arity pipeline stage call should type-check and run\nstderr={stderr}"
    );
    assert!(
        stdout_text(&output).contains("15"),
        "expected pair(5, 10) = 15, got: {}",
        stdout_text(&output)
    );
}

/// Regression: a plain (non-pipeline) over-arity call is still rejected and
/// the diagnostic does *not* carry the pipe-injection note.
#[test]
fn e1301_rejects_plain_over_arity_without_pipe_note() {
    let output = run_interp(
        "f57_006_plain_over",
        r#"pair a: Int b: Int = a + b => :Int
x <= pair(10, 20, 30)
stdout(x.toString())
"#,
    );
    assert_e1301(&output, "plain over arity");
    assert!(
        !stderr_text(&output).contains("piped value counts as the first argument"),
        "a non-pipeline call must not carry the pipe-injection note, got: {}",
        stderr_text(&output)
    );
}

// ── [E1506] type index-shift (review follow-up) ─────────────────────
// When a pipeline injects the piped value as the implicit first argument, the
// written arguments fill param slots 1.. — so a written value must be type-
// checked against the slot it *actually* fills, and the injected value against
// param 0. Before this follow-up, only arity ([E1301]) counted the injection;
// the type check ([E1506]) still compared written args against params 0.., so
// `5 => need(10)` (need: Int, Str) silently let `10` satisfy `a: Int` while it
// really fills `b: Str`.

/// A written argument is checked against the param it fills after the pipeline
/// injection shifts it. `5 => need(10)` lowers to `need(5, 10)`, so `10` is
/// checked against `b: Str` → [E1506] on argument 2.
#[test]
fn e1506_pipeline_injection_shifts_written_arg_type_check() {
    let output = run_interp(
        "f57_006_typeshift_written",
        r#"need a: Int b: Str = b => :Str
5 => need(10)
"#,
    );
    assert!(
        !output.status.success(),
        "type-shifted mismatch must be rejected\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("[E1506]"),
        "expected [E1506], got: {}",
        stderr_text(&output)
    );
}

/// The injected first argument itself is type-checked against param 0.
/// `"s" => need("x")` injects a Str where `a: Int` is expected → [E1506] on
/// argument 1, flagged as the piped value.
#[test]
fn e1506_pipeline_injected_first_arg_type_checked() {
    let output = run_interp(
        "f57_006_typeshift_injected",
        r#"need a: Int b: Str = b => :Str
"s" => need("x")
"#,
    );
    assert!(
        !output.status.success(),
        "injected-arg mismatch must be rejected\nstderr={}",
        stderr_text(&output)
    );
    let e = stderr_text(&output);
    assert!(e.contains("[E1506]"), "expected [E1506], got: {e}");
    assert!(
        e.contains("piped value"),
        "expected the pipe-injection note, got: {e}"
    );
}

/// No false reject: when the injected and written types both match the shifted
/// slots, the call type-checks and runs. `5 => need("x")` → `need(5, "x")`:
/// `a: Int` OK, `b: Str` OK.
#[test]
fn e1506_pipeline_injection_exact_types_ok() {
    let output = run_interp(
        "f57_006_typeshift_ok",
        r#"need a: Int b: Str = b => :Str
5 => need("x") => stdout(_)
"#,
    );
    let e = stderr_text(&output);
    assert!(
        !e.contains("[E1506]"),
        "exact-type pipeline call must not be rejected, got: {e}"
    );
    assert!(
        output.status.success(),
        "exact-type pipeline call should type-check and run\nstderr={e}"
    );
    assert!(
        stdout_text(&output).contains('x'),
        "expected 'x', got: {}",
        stdout_text(&output)
    );
}
