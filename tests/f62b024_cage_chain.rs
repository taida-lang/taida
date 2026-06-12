// F62B-024: the CageBuilder chain — host-call pipelines without nesting.
//
//   Cage[subject]()                       — opens a builder (capability + empty steps)
//   InCage[builder, method, args]()       — pushes one step, returns the extended builder
//   Uncage[builder, method, Out]()        — pushes the final (arg-less) step and fires
//                                           the host cage as Async[Out]
//
// The chain builds a description; the host call is issued exactly once, at
// `Uncage`. The wire form (one HostCall envelope with a steps array) is the
// same as the direct `Cage[subject, HostCall[...]]()`, so the two forms are
// equivalent against the same interpreter host fixture.
//
// The builder is a plain pack (deliberately no `__type`), so unmolding one
// is the plain-pack gorilla settled by the unmold rules.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::fs;
use std::process::{Command, Output};
use taida::interpreter::{AsyncStatus, HostCallMockStep, Interpreter, Value};

fn eval_with_fixture(source: &str, capability: &str, steps: Vec<HostCallMockStep>) -> Value {
    let (program, parse_errors) = taida::parser::parse(source);
    assert!(parse_errors.is_empty(), "parse errors: {:?}", parse_errors);
    let mut interpreter = Interpreter::new();
    interpreter.set_host_capability_mock_steps(capability, steps);
    interpreter
        .eval_program(&program)
        .expect("host capability fixture should evaluate")
}

fn fixture_get_decode() -> Vec<HostCallMockStep> {
    vec![
        HostCallMockStep {
            method: "get".to_string(),
            args: vec![Value::str("k".to_string())],
            result: Value::str("raw".to_string()),
        },
        HostCallMockStep {
            method: "decode".to_string(),
            args: Vec::new(),
            result: Value::str("value".to_string()),
        },
    ]
}

/// The chain form resolves to the same value as the direct
/// `Cage[subject, HostCall[...]]()` against the same fixture.
#[test]
fn chain_matches_direct_form() {
    let direct = r#"Kind <= "mock/kind"
cap <= HostCapability["CAP", Kind]()
Cage[cap, HostCall[@[HostStep["get", @["k"]](), HostStep["decode", @[]]()], Str]()]() >=> out
out
"#;
    let chain = r#"Kind <= "mock/kind"
cap <= HostCapability["CAP", Kind]()
Cage[cap]() => InCage[_, "get", @["k"]]() => Uncage[_, "decode", Str]() >=> out
out
"#;
    let direct_result = eval_with_fixture(direct, "CAP", fixture_get_decode());
    let chain_result = eval_with_fixture(chain, "CAP", fixture_get_decode());
    assert_eq!(direct_result, Value::str("value".to_string()));
    assert_eq!(
        chain_result, direct_result,
        "chain and direct forms must resolve identically"
    );
}

/// Builders are first-class values with value semantics: extending a base
/// chain does not mutate it. Building a longer chain from `base` and then
/// firing `base` itself must send only base's steps — a mutating push
/// would make the fired step list mismatch the fixture.
#[test]
fn builder_extension_does_not_mutate_base() {
    // `<=` cannot mix with `=>` in one statement (E0301), so chains bind
    // through the pipeline-final-identifier assignment form.
    let source = r#"Kind <= "mock/kind"
cap <= HostCapability["CAP", Kind]()
Cage[cap]() => InCage[_, "get", @["k"]]() => base
base => InCage[_, "bind", @[1]]() => longer
base => Uncage[_, "decode", Str]() >=> out
out
"#;
    let result = eval_with_fixture(source, "CAP", fixture_get_decode());
    assert_eq!(
        result,
        Value::str("value".to_string()),
        "firing the base chain must send base's steps only"
    );
}

/// A wrong chain (extra step) is rejected by the fixture — proving the
/// steps actually flow into the envelope.
#[test]
fn chain_steps_flow_into_the_envelope() {
    let source = r#"Kind <= "mock/kind"
cap <= HostCapability["CAP", Kind]()
Cage[cap]() => InCage[_, "get", @["k"]]() => InCage[_, "extra", @[]]() => Uncage[_, "decode", Str]()
"#;
    let result = eval_with_fixture(source, "CAP", fixture_get_decode());
    let Value::Async(async_value) = result else {
        panic!("Uncage should return Async, got {:?}", result);
    };
    assert_eq!(
        async_value.status,
        AsyncStatus::Rejected,
        "a step-count mismatch must reject"
    );
}

// ── checker rules ────────────────────────────────────────────────

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

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// InCage / Uncage demand a CageBuilder first argument.
#[test]
fn chain_molds_reject_non_builder_first_arg() {
    let output = run_interp("f62b024_non_builder", "x <= 5\nInCage[x, \"get\", @[]]()\n");
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1517]") && stderr_text(&output).contains("CageBuilder"),
        "expected builder-arg rejection, got: {}",
        stderr_text(&output)
    );
}

/// Chain methods are compile-time strings, mirroring HostStep.
#[test]
fn chain_method_must_be_compile_time_str() {
    let output = run_interp(
        "f62b024_dyn_method",
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "Join[@[\"get\"], \"\"]() >=> m\n",
            "Cage[db]() => InCage[_, m, @[]]()\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E3603]"),
        "expected compile-time method rejection, got: {}",
        stderr_text(&output)
    );
}

/// InCage args must be a wire-encodable list ([E3601], like HostStep).
#[test]
fn chain_args_must_be_wired() {
    let output = run_interp(
        "f62b024_unwired",
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "Cage[db]() => InCage[_, \"get\", @[_ x: Int = x]]()\n",
        ),
    );
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E3601]"),
        "expected Wired rejection, got: {}",
        stderr_text(&output)
    );
}

/// The builder-form subject must be a HostCapability.
#[test]
fn builder_subject_must_be_host_capability() {
    let output = run_interp("f62b024_bad_subject", "Cage[42]()\n");
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1517]"),
        "expected subject rejection, got: {}",
        stderr_text(&output)
    );
}

/// Unmolding a builder is the plain-pack gorilla (the chain's value comes
/// out at `Uncage`, not by unmolding the description).
#[test]
fn unmolding_a_builder_is_gorilla() {
    let output = run_interp(
        "f62b024_builder_unmold",
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "b <= Cage[db]()\n",
            "b >=> x\n",
            "stdout(\"unreachable\")\n",
        ),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1545]") && stderr.contains("><"),
        "expected the plain-pack gorilla, got: {stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("unreachable"));
}

// ── native backend ───────────────────────────────────────────────

/// The chain lowers natively and resolves to the deterministic session-less
/// rejection (the same one the direct host-cage form produces).
#[test]
fn native_chain_resolves_to_sessionless_rejection() {
    let dir = unique_temp_dir("f62b024_native");
    let src = dir.join("main.td");
    write_file(
        &src,
        concat!(
            "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
            "Cage[db]() => InCage[_, \"get\", @[\"k\"]]() => Uncage[_, \"decode\", Str]() >=> out\n",
            "stdout(out)\n",
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
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        stderr.contains("host capabilities are not available"),
        "expected the session-less rejection, got: {stderr}"
    );
}
