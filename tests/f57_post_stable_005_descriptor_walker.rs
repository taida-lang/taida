// POST-STABLE-005 regression: the build-descriptor runtime-use pass
// (`[E1532]`) must also walk class-like definitions — field default
// expressions and method bodies — and function parameter defaults, not only
// top-level statements and plain function bodies.
//
// Before this fix `check_descriptor_use_in_stmt` fell through `ClassLikeDef`
// (and never inspected parameter defaults), so a descriptor used as a runtime
// value inside a type definition was a False Negative (over-allow). These
// tests pin the reject for both class-like positions, plus a non-descriptor
// field default that must keep type-checking (the new walker arm must not
// over-reject ordinary values).
//
// Parameter defaults are walked defensively too, but the parser never
// produces them (Taida has no parameter-default syntax), so there is no
// reachable reject fixture for that arm — it guards AST completeness only.
//
// Like the other `[E1532]` suites these drive the interpreter entry point
// (`taida <FILE>`); see `tests/f55_s3_descriptor_diag.rs`.

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

fn assert_e1532(output: &Output, label: &str) {
    assert!(
        !output.status.success(),
        "{label}: descriptor runtime use must be rejected\nstdout={}\nstderr={}",
        stdout_text(output),
        stderr_text(output)
    );
    assert!(
        stderr_text(output).contains("[E1532]"),
        "{label}: expected [E1532], got: {}",
        stderr_text(output)
    );
}

/// Reject #1: a descriptor in a class-like *field default* expression is a
/// runtime use — the type definition is not the descriptor build path.
#[test]
fn e1532_rejects_descriptor_in_class_like_field_default() {
    let output = run_interp(
        "f57_005_field_default",
        r#"serverMain <= "x"
Config = @(u <= BuildUnit(name <= "u", target <= "native", entry <= serverMain))
stdout("done")
"#,
    );
    assert_e1532(&output, "class-like field default");
}

/// Reject #2: a descriptor used inside a class-like *method body* is a runtime
/// use, exactly as inside a top-level function body.
#[test]
fn e1532_rejects_descriptor_in_class_like_method_body() {
    let output = run_interp(
        "f57_005_method_body",
        r#"serverMain <= "x"
unit <= BuildUnit(name <= "u", target <= "native", entry <= serverMain)
Config = @(
  describe self = Str[unit]() => :Str
)
stdout("done")
"#,
    );
    assert_e1532(&output, "class-like method body");
}

/// Guard: a class-like field default that is *not* a descriptor must keep
/// type-checking and running — the new walker arm must not over-reject
/// ordinary runtime values.
#[test]
fn e1532_allows_non_descriptor_class_like_field_default() {
    let output = run_interp(
        "f57_005_plain_field_default",
        r#"Config = @(count <= 0)
stdout("ok")
"#,
    );
    let stderr = stderr_text(&output);
    assert!(
        !stderr.contains("[E1532]"),
        "plain field default must not trigger [E1532], got: {stderr}"
    );
    assert!(
        output.status.success(),
        "a class-like definition with a plain (non-descriptor) field default must type-check and run\nstderr={stderr}"
    );
    assert!(
        stdout_text(&output).contains("ok"),
        "expected program output, got: {}",
        stdout_text(&output)
    );
}
