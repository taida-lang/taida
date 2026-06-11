// F62B-025: pipeline application closes over exactly two rules.
//
//   Rule 1 — the stage contains `_` (one at most, E1543): the piped value
//   is injected syntactically at the placeholder.
//   Rule 2 — no `_`: the stage is evaluated as written; a function result
//   receives the piped value, anything else is E1544.
//
// These tests pin the semantics that replaced the legacy implicit
// first-argument injection (`5 => f(3)` running as `f(5, 3)`, removed) and
// the inline empty-slot compositionality bug (`5 => add(, 3)` used to build
// a poison closure instead of evaluating the partial application and
// applying 5).
//
// Like the other semantic suites these drive the interpreter entry point
// (`taida <FILE>`); backend parity for the runnable matrix is covered by
// `examples/compile_f62b025_pipe_semantics.td` through the parity gate.

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

const ADD: &str = "add x: Int y: Int = x + y => :Int\n";
const DOUBLE: &str = "double n: Int = n * 2 => :Int\n";

/// Rule 2 compositionality: an inline empty-slot partial application
/// evaluates to a closure and the piped value is applied to it —
/// `5 => add(, 3)` ≡ `f <= add(, 3)` + `5 => f` = 8.
#[test]
fn rule2_inline_partial_application_applies_piped_value() {
    let output = run_interp(
        "f62b025_inline_partial",
        &format!("{ADD}5 => add(, 3) => r\nstdout(r.toString())\n"),
    );
    assert!(
        output.status.success(),
        "inline partial stage must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains('8'),
        "expected add(5, 3) = 8, got: {}",
        stdout_text(&output)
    );
}

/// Rule 2 compositionality, variable form: binding the partial application
/// first must give the same result as the inline form.
#[test]
fn rule2_closure_variable_stage_applies_piped_value() {
    let output = run_interp(
        "f62b025_closure_var",
        &format!("{ADD}f <= add(, 3)\n5 => f => r\nstdout(r.toString())\n"),
    );
    assert!(
        output.status.success(),
        "closure-variable stage must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains('8'),
        "expected f(5) = 8, got: {}",
        stdout_text(&output)
    );
}

/// Rule 1: a single `_` marks the injection position.
#[test]
fn rule1_single_placeholder_injects() {
    let output = run_interp(
        "f62b025_inject",
        &format!("{ADD}5 => add(_, 3) => r\nstdout(r.toString())\n"),
    );
    assert!(output.status.success());
    assert!(
        stdout_text(&output).contains('8'),
        "expected add(5, 3) = 8, got: {}",
        stdout_text(&output)
    );
}

/// Rule 1 + empty slots: `_` injects while the empty slot stays a hole, so
/// `5 => addThree(, _, 3)` is a positional injection producing a closure
/// that still waits for its first argument.
#[test]
fn rule1_mixed_hole_and_placeholder_is_positional_injection() {
    let output = run_interp(
        "f62b025_mixed",
        "addThree x: Int y: Int z: Int = x + y + z => :Int\n\
         5 => addThree(, _, 3) => f\n\
         f(100) => r\n\
         stdout(r.toString())\n",
    );
    assert!(
        output.status.success(),
        "positional injection must produce a callable closure\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("108"),
        "expected addThree(100, 5, 3) = 108, got: {}",
        stdout_text(&output)
    );
}

/// E1543: two `_` in one stage is a static reject — `_` marks where the
/// single piped value goes, holes are written as empty slots.
#[test]
fn e1543_rejects_two_placeholders_in_one_stage() {
    let output = run_interp(
        "f62b025_two_placeholders",
        &format!("{ADD}5 => add(_, _) => r\nstdout(r.toString())\n"),
    );
    assert!(
        !output.status.success(),
        "two placeholders in one stage must be rejected"
    );
    assert!(
        stderr_text(&output).contains("[E1543]"),
        "expected [E1543], got: {}",
        stderr_text(&output)
    );
}

/// Rule 2: bare function names and `f(_)` stay valid (`5 => double` and
/// `5 => double(_)` are both 10).
#[test]
fn rule2_bare_name_and_explicit_placeholder_apply() {
    let output = run_interp(
        "f62b025_bare_name",
        &format!(
            "{DOUBLE}5 => double => a\nstdout(a.toString())\n5 => double(_) => b\nstdout(b.toString())\n"
        ),
    );
    assert!(output.status.success());
    let out = stdout_text(&output);
    assert_eq!(out.matches("10").count(), 2, "expected two 10s, got: {out}");
}

/// E1544: a `_`-free call stage is evaluated as written — `double()` is an
/// immediate call producing 0 (an Int), so piping into it is a static
/// non-function reject. The legacy `5 => double()` ≡ `double(5)` form is gone.
#[test]
fn e1544_rejects_zero_arg_call_stage() {
    let output = run_interp(
        "f62b025_zero_arg_call",
        &format!("{DOUBLE}5 => double() => r\nstdout(r.toString())\n"),
    );
    assert!(
        !output.status.success(),
        "zero-arg call stage must be rejected"
    );
    assert!(
        stderr_text(&output).contains("[E1544]"),
        "expected [E1544], got: {}",
        stderr_text(&output)
    );
}

/// E1544: the legacy implicit first-argument injection is gone —
/// `5 => add(3)` evaluates `add(3)` (an Int via default completion), which
/// is not a function.
#[test]
fn e1544_rejects_legacy_first_arg_injection() {
    let output = run_interp(
        "f62b025_legacy_injection",
        &format!("{ADD}5 => add(3) => r\nstdout(r.toString())\n"),
    );
    assert!(
        !output.status.success(),
        "legacy injection form must be rejected"
    );
    assert!(
        stderr_text(&output).contains("[E1544]"),
        "expected [E1544], got: {}",
        stderr_text(&output)
    );
}

/// Rule 1 reaches method arguments: `5 => nums.get(_)` injects into the
/// method argument position (this was a runtime [E1502] before F62B-025).
#[test]
fn rule1_placeholder_in_method_argument() {
    let output = run_interp(
        "f62b025_method_arg",
        "nums <= @[30, 10, 20]\n\
         1 => nums.get(_) => v\n\
         stdout(v.getOrDefault(-1).toString())\n",
    );
    assert!(
        output.status.success(),
        "method-argument placeholder must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("10"),
        "expected nums.get(1) = 10, got: {}",
        stdout_text(&output)
    );
}

/// Rule 1 placeholder typing: `_` carries the piped value's type, so a
/// bare operator stage like `5 => _ + 1` type-checks and runs.
#[test]
fn rule1_placeholder_carries_piped_type_in_operator_stage() {
    let output = run_interp(
        "f62b025_operator_stage",
        "5 => _ + 1 => r\nstdout(r.toString())\n",
    );
    assert!(
        output.status.success(),
        "operator stage with `_` must type-check\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains('6'),
        "expected 5 + 1 = 6, got: {}",
        stdout_text(&output)
    );
}

/// C13-1 bind-and-forward stays intact: an intermediate `=> name` binds the
/// value, and a later stage that references the binding is evaluated as
/// written (no piped-value application, no E1544).
#[test]
fn bind_forward_reference_stage_unaffected() {
    let output = run_interp(
        "f62b025_bind_forward",
        &format!("{ADD}1 => add(3, _) => bound => stdout(bound.toString())\n"),
    );
    assert!(
        output.status.success(),
        "bind-forward reference stage must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains('4'),
        "expected add(3, 1) = 4, got: {}",
        stdout_text(&output)
    );
}

/// Regression (kept from the retired POST-STABLE-006 suite): a plain
/// non-pipeline over-arity call is still a static [E1301].
#[test]
fn e1301_rejects_plain_over_arity() {
    let output = run_interp(
        "f62b025_plain_over_arity",
        &format!("{ADD}x <= add(10, 20, 30)\nstdout(x.toString())\n"),
    );
    assert!(
        !output.status.success(),
        "plain over-arity call must be rejected"
    );
    assert!(
        stderr_text(&output).contains("[E1301]"),
        "expected [E1301], got: {}",
        stderr_text(&output)
    );
}
