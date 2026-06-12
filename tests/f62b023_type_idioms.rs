// F62B-023: idioms for pinning types onto bindings.
//
//   (c) unmold bindings accept a target type annotation, symmetric in both
//       directions: `name: Type <=< expr` and `expr >=> name: Type`.
//       The annotation is checker-only: it is validated against the
//       unmolded type (mismatch is a type error) and becomes the binding
//       type, sharpening `Unknown` from unresolved cross-module types.
//
// These tests drive the interpreter entry point (`taida <FILE>`); backend
// parity for the runnable forms is covered by
// `examples/compile_f62b023_typed_unmold.td` through the parity gate.

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

/// (c) `name: Type <=< expr` parses and binds with the annotated type —
/// the arithmetic after the binding proves the checker accepted Int.
#[test]
fn typed_unmold_backward_binds_with_annotation() {
    let output = run_interp(
        "f62b023_typed_backward",
        "half: Int <=< Div[10, 2]()\nstdout((half + 1).toString())\n",
    );
    assert!(
        output.status.success(),
        "typed <=< binding must run\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "6\n");
}

/// (c) symmetric forward form: `expr >=> name: Type`.
#[test]
fn typed_unmold_forward_binds_with_annotation() {
    let output = run_interp(
        "f62b023_typed_forward",
        "Div[20, 4]() >=> fifth: Int\nstdout((fifth * 2).toString())\n",
    );
    assert!(
        output.status.success(),
        "typed >=> binding must run\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "10\n");
}

/// (c) the annotation is checked: annotating an Int unmold as Str is a
/// type error, mirroring typed-assignment mismatch handling.
#[test]
fn typed_unmold_annotation_mismatch_is_type_error() {
    let output = run_interp(
        "f62b023_typed_mismatch",
        "half: Str <=< Div[10, 2]()\nstdout(half)\n",
    );
    assert!(
        !output.status.success(),
        "Str annotation over an Int unmold must be rejected"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("Type mismatch in unmold binding to 'half'"),
        "expected unmold-binding mismatch diagnostic, got: {stderr}"
    );
}

/// (c) the forward direction validates the annotation too.
#[test]
fn typed_unmold_forward_annotation_mismatch_is_type_error() {
    let output = run_interp(
        "f62b023_typed_forward_mismatch",
        "Div[10, 2]() >=> half: Str\nstdout(half)\n",
    );
    assert!(
        !output.status.success(),
        "Str annotation over an Int unmold must be rejected"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("Type mismatch in unmold binding to 'half'"),
        "expected unmold-binding mismatch diagnostic, got: {stderr}"
    );
}

/// (c) the single-direction constraint survives the typed form:
/// `name: T <=< expr >=> other` is still E0302.
#[test]
fn typed_unmold_backward_rejects_mixed_direction() {
    let output = run_interp(
        "f62b023_typed_e0302",
        "half: Int <=< Div[10, 2]() >=> bad\n",
    );
    assert!(
        !output.status.success(),
        "mixed >=> after typed <=< must be rejected"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("E0302"),
        "expected E0302 single-direction violation, got: {stderr}"
    );
}

/// (c) untyped forms are untouched.
#[test]
fn untyped_unmold_forms_unchanged() {
    let output = run_interp(
        "f62b023_untyped_compat",
        "third <=< Div[9, 3]()\nDiv[8, 2]() >=> quarter\nstdout(third.toString())\nstdout(quarter.toString())\n",
    );
    assert!(
        output.status.success(),
        "untyped unmold forms must keep working\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "3\n4\n");
}

// ── (b) list type aliases: `Name = @[ElemType]` ─────────────────────

fn run_interp_files(label: &str, files: &[(&str, &str)], entry: &str) -> Output {
    let dir = unique_temp_dir(label);
    for (name, source) in files {
        write_file(&dir.join(name), source);
    }
    let output = Command::new(taida_bin())
        .arg(dir.join(entry))
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    output
}

/// (b) the headline idiom: an alias names a list-of-packs type and pins an
/// empty literal through an annotated binding.
#[test]
fn list_type_alias_pins_empty_literal() {
    let output = run_interp(
        "f62b023_alias_basic",
        "Pairs = @[@(name: Str, value: Str)]\nemptyPairs: Pairs <= @[]\nstdout(emptyPairs.length().toString())\n",
    );
    assert!(
        output.status.success(),
        "alias-annotated empty list must check\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "0\n");
}

/// (b) aliases expand in parameter and return annotations.
#[test]
fn list_type_alias_in_function_annotations() {
    let output = run_interp(
        "f62b023_alias_fn",
        "Pairs = @[@(name: Str, value: Str)]\ncountPairs ps: Pairs = ps.length() => :Int\nps: Pairs <= @[@(name <= \"a\", value <= \"b\")]\nstdout(countPairs(ps).toString())\n",
    );
    assert!(
        output.status.success(),
        "alias in fn annotations must check\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "1\n");
}

/// (b) alias chains expand at registration (`Grid = @[Row]`).
#[test]
fn list_type_alias_chain_expands() {
    let output = run_interp(
        "f62b023_alias_chain",
        "Row = @[Int]\nGrid = @[Row]\ng: Grid <= @[@[1, 2], @[3]]\nstdout(g.length().toString())\n",
    );
    assert!(
        output.status.success(),
        "alias chain must check\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "2\n");
}

/// (b) alias names join the E1501 collision space — both definition orders.
#[test]
fn list_type_alias_collision_is_e1501_both_orders() {
    for (label, source) in [
        (
            "f62b023_alias_collide_1",
            "Pairs = @[Int]\nPairs = @(x: Int)\n",
        ),
        (
            "f62b023_alias_collide_2",
            "Pairs = @(x: Int)\nPairs = @[Int]\n",
        ),
    ] {
        let output = run_interp(label, source);
        assert!(
            !output.status.success(),
            "{label}: redefinition must be rejected"
        );
        let stderr = stderr_text(&output);
        assert!(
            stderr.contains("E1501"),
            "{label}: expected E1501 collision, got: {stderr}"
        );
    }
}

/// (b) a mismatch through an alias reports the expanded structural type.
#[test]
fn list_type_alias_mismatch_reports_structural_type() {
    let output = run_interp(
        "f62b023_alias_mismatch",
        "Pairs = @[@(name: Str, value: Str)]\nbad: Pairs <= @[1, 2]\n",
    );
    assert!(
        !output.status.success(),
        "Int list against pack alias must be rejected"
    );
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("expected @[@(name: Str, value: Str)]"),
        "expected the alias to expand in the diagnostic, got: {stderr}"
    );
}

/// (b) aliases cross module boundaries through import lists.
#[test]
fn list_type_alias_imports_across_modules() {
    let output = run_interp_files(
        "f62b023_alias_xmodule",
        &[
            (
                "lib.td",
                "Pairs = @[@(name: Str, value: Str)]\n<<< @(Pairs)\n",
            ),
            (
                "main.td",
                ">>> ./lib.td => @(Pairs)\nps: Pairs <= @[@(name <= \"x\", value <= \"y\")]\nstdout(ps.length().toString())\n",
            ),
        ],
        "main.td",
    );
    assert!(
        output.status.success(),
        "imported alias must check\nstderr={}",
        stderr_text(&output)
    );
    assert_eq!(stdout_text(&output), "1\n");
}
