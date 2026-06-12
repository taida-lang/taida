// F62B-001: module functions carried across the import boundary must keep
// resolving their defining module's symbols at any call-chain depth.
//
// A module function's definition-time closure snapshot does not contain
// siblings defined after it, and the export-time closure enrichment only
// reaches two levels. Deep chains (trampoline retargets for tail calls,
// nested normal calls, arm-tail chains) used to degrade to the truncated
// definition-time closure and die with `Undefined variable` — and module-
// local JSON schemas vanished the same way. The fix attaches the defining
// module's symbol table to every module function and re-attaches it to any
// sibling resolved through it, so resolution is self-sustaining.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::fs;
use std::process::{Command, Output};

/// Write `mod.td` + `use.td` into a temp dir and run `use.td`.
fn run_pair(label: &str, module_src: &str, use_src: &str) -> Output {
    let dir = unique_temp_dir(label);
    write_file(&dir.join("mod.td"), module_src);
    let use_path = dir.join("use.td");
    write_file(&use_path, use_src);
    let output = Command::new(taida_bin())
        .arg(&use_path)
        .current_dir(&dir)
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

const MUTUAL_MOD: &str = r#"entry n: Int =
  pingA(n)
=> :Str

pingA n: Int =
  | n < 1 |> "done"
  | _ |> pingB(n - 1)
=> :Str

pingB n: Int =
  | n < 1 |> "done"
  | _ |> pingA(n - 1)
=> :Str

<<< @(entry)
"#;

/// The original reproduction: a wrapper entry into a mutual tail-recursive
/// pair, imported and called from another file. The second retarget used to
/// die with `Undefined variable: 'pingB'`.
#[test]
fn imported_wrapper_mutual_tail_recursion_runs() {
    let output = run_pair(
        "f62b001_wrapper_mutual",
        MUTUAL_MOD,
        ">>> ./mod.td => @(entry)\nstdout(entry(5))\n",
    );
    assert!(
        output.status.success(),
        "imported wrapper mutual recursion must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("done"),
        "expected 'done', got: {}",
        stdout_text(&output)
    );
}

/// Deep trampoline: 200k mutual hops through the import boundary must
/// neither lose the sibling nor overflow the stack.
#[test]
fn imported_mutual_tail_recursion_deep_hops() {
    let output = run_pair(
        "f62b001_deep_hops",
        MUTUAL_MOD,
        ">>> ./mod.td => @(entry)\nstdout(entry(200000))\n",
    );
    assert!(
        output.status.success(),
        "deep mutual hops must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(stdout_text(&output).contains("done"));
}

/// Chained arm-tail calls (depth 4) ending in a module-local JSON schema:
/// both the late-defined sibling functions and the module's typedef scope
/// must survive every retarget.
#[test]
fn imported_arm_tail_chain_keeps_functions_and_schemas() {
    let module = r#"Payload = @(kind: Str, value: Int)

stepFour n: Int =
  p <= JSON[`{"kind": "leaf", "value": ${n}}`, Payload]()
  p >=> payload
  payload.value
=> :Int

stepThree n: Int =
  | n > 0 |> stepFour(n)
  | _ |> 0
=> :Int

stepTwo n: Int =
  | n > 0 |> stepThree(n)
  | _ |> 0
=> :Int

stepOne n: Int =
  | n > 0 |> stepTwo(n)
  | _ |> 0
=> :Int

<<< @(stepOne)
"#;
    let output = run_pair(
        "f62b001_arm_chain_schema",
        module,
        ">>> ./mod.td => @(stepOne)\nstdout(stepOne(42).toString())\n",
    );
    assert!(
        output.status.success(),
        "arm-tail chain with module-local schema must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("42"),
        "expected 42, got: {}",
        stdout_text(&output)
    );
}

/// Non-tail nested mutual calls (`1 + other(n - 1)`) also walk sibling
/// closures — depth 3+ used to hit the truncated definition-time closure.
#[test]
fn imported_non_tail_mutual_nest_resolves_at_depth() {
    let module = r#"deepNonTail n: Int =
  | n < 1 |> 0
  | _ |> 1 + deepInner(n - 1)
=> :Int

deepInner n: Int =
  | n < 1 |> 0
  | _ |> 1 + deepNonTail(n - 1)
=> :Int

<<< @(deepNonTail)
"#;
    let output = run_pair(
        "f62b001_nontail_nest",
        module,
        ">>> ./mod.td => @(deepNonTail)\nstdout(deepNonTail(6).toString())\n",
    );
    assert!(
        output.status.success(),
        "non-tail mutual nest must run\nstderr={}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains('6'),
        "expected 6, got: {}",
        stdout_text(&output)
    );
}
