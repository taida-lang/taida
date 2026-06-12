// F62B-028 [E1546]: a bare `Name[args]` (no `()`) in a value position is a
// missing cast, not an accepted shorthand. `[]` puts the value in the mold,
// `()` casts it — the two-step is the design; the cast is not optional.
//
// Type positions stay legal: annotations parse through the type grammar,
// and a bare mold inside another mold's `[...]` arguments is a type
// reference (schema / Out slots like `JSON[x, Lax[Int]]()`).

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

fn assert_e1546(label: &str, source: &str) {
    let output = run_interp(label, source);
    assert!(!output.status.success(), "{label} must be rejected");
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1546]"),
        "{label}: expected E1546, got: {stderr}"
    );
}

/// Bare molds are rejected in every value position.
#[test]
fn bare_mold_rejected_in_value_positions() {
    assert_e1546("f62b028_assign", "t <= Trim[\"  hi  \"]\nstdout(t)\n");
    assert_e1546("f62b028_pipe", "\"  hi  \" => Trim[_] => t\nstdout(t)\n");
    assert_e1546(
        "f62b028_arg",
        "f s: Str = s => :Str\nstdout(f(Trim[\"  x  \"]))\n",
    );
    assert_e1546("f62b028_unmold", "Sort[@[2, 1]] >=> xs\nstdout(xs)\n");
}

/// The `()` forms keep working everywhere the bare forms were rejected.
#[test]
fn cast_forms_unaffected() {
    let output = run_interp(
        "f62b028_cast_ok",
        concat!(
            "t <= Trim[\"  hi  \"]()\n",
            "stdout(t)\n",
            "\"  pad  \" => Trim[_]() => t2\n",
            "stdout(t2)\n",
            "Sort[@[2, 1]]() >=> xs\n",
            "stdout(xs.length().toString())\n",
        ),
    );
    assert!(
        output.status.success(),
        "cast forms must run\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "hi\npad\n2\n");
}

/// A bare mold inside another mold's `[...]` arguments is a type
/// reference and stays legal (the JSON schema slot is the canonical use).
#[test]
fn bare_mold_as_type_reference_in_mold_args_stays_legal() {
    let output = run_interp(
        "f62b028_type_ref",
        concat!(
            "Box = @(a: Int)\n",
            "raw <= \"{\\\"a\\\": 1}\"\n",
            "JSON[raw, Box]() >=> v\n",
            "stdout(v.a.toString())\n",
        ),
    );
    assert!(
        output.status.success(),
        "type reference in mold args must stay legal\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "1\n");
}

/// Generic type references in mold args (`Lax[Int]` style) stay legal too.
#[test]
fn bare_generic_mold_in_mold_args_stays_legal() {
    let output = run_interp(
        "f62b028_generic_ref",
        concat!(
            "raw <= \"[1, 2, 3]\"\n",
            "JSON[raw, @[Int]]() >=> xs\n",
            "stdout(xs.length().toString())\n",
        ),
    );
    assert!(
        output.status.success(),
        "list type in schema slot must stay legal\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "3\n");
}

/// Type annotations are a different grammar and never collide.
#[test]
fn type_annotations_unaffected() {
    let output = run_interp(
        "f62b028_annotation",
        "x: Lax[Int] <= Div[10, 2]()\nstdout(x.hasValue().toString())\n",
    );
    assert!(
        output.status.success(),
        "Lax[Int] annotation must stay legal\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "true\n");
}
