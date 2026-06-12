// Phase 1 review follow-ups (Codex second-opinion findings, all fixed
// in-track):
//
// - C-1: REPL evaluates on a large-stack thread, so deep recursion hits
//   the depth diagnostic instead of a Rust stack overflow.
// - C-2: overlapping mutual cycles (a↔b and a↔c) union into ONE
//   dispatcher family — two dispatchers used to call each other through
//   plain wrapper calls and abort at depth on native.
// - C-4: a `_` inside a stage that references a `=> name` binding is a
//   uniform [E1543] on every surface (it was checker-silent, a confusing
//   interp error, a JS ReferenceError, and a native silent wrong answer).
// - C-5: re-binding the same `=> name` twice in one pipeline works on JS
//   (fresh synthetic consts, the JS sibling of F57B-007).

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::process::Command;

#[test]
fn repl_deep_recursion_stays_diagnostic() {
    use std::io::Write;
    let mut child = Command::new(taida_bin())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"burn n: Int = | n < 1 |> 0 | _ |> 1 + burn(n - 1) => :Int\nburn(8000)\n")
        .expect("write repl input");
    let out = child.wait_with_output().expect("repl run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("8000"),
        "REPL must complete depth-8000 recursion on the eval thread, got: {stdout}"
    );
    assert!(
        out.status.code().is_some(),
        "REPL must exit normally, not die on a signal: {:?}",
        out.status
    );
}

#[test]
fn overlapping_mutual_cycles_union_into_one_dispatcher() {
    let dir = unique_temp_dir("f62rev_overlap");
    let td = dir.join("main.td");
    write_file(
        &td,
        "stepA n: Int =\n  | n < 1 |> 0\n  | n < 2 |> stepB(n - 1)\n  | _ |> stepC(n - 1)\n=> :Int\n\nstepB n: Int =\n  | n < 1 |> 0\n  | _ |> stepA(n - 1)\n=> :Int\n\nstepC n: Int =\n  | n < 1 |> 0\n  | _ |> stepA(n - 1)\n=> :Int\n\nstdout(stepA(600000).toString())\n",
    );
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(&td)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "overlapping tail-only cycles must compile (unioned family)\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&bin).output().expect("run");
    assert!(
        out.status.success(),
        "600k-deep overlapping cycles must not abort: {:?}",
        out.status
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains('0'));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn placeholder_in_bound_reference_stage_is_uniform_e1543() {
    let dir = unique_temp_dir("f62rev_c4");
    let td = dir.join("main.td");
    write_file(
        &td,
        "add a: Int  b: Int =\n  a + b\n=> :Int\n5 => v => add(_, v) => r\nstdout(r.toString())\n",
    );
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[E1543]"),
        "expected uniform [E1543], got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn js_rebinding_same_pipeline_name_matches_other_backends() {
    let dir = unique_temp_dir("f62rev_c5");
    let td = dir.join("main.td");
    write_file(
        &td,
        "add a: Int  b: Int =\n  a + b\n=> :Int\nmul a: Int  b: Int =\n  a * b\n=> :Int\n5 => a => add(a, 1) => a => mul(a, 2) => r\nstdout(r.toString())\n",
    );
    let interp = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(interp.status.success());
    assert!(String::from_utf8_lossy(&interp.stdout).contains("12"));

    // JS backend (skip silently when node is unavailable).
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("SKIP: node unavailable");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let mjs = dir.join("main.mjs");
    let build = Command::new(taida_bin())
        .args(["build", "js"])
        .arg(&td)
        .arg("-o")
        .arg(&mjs)
        .output()
        .expect("js build");
    assert!(build.status.success());
    let out = Command::new("node").arg(&mjs).output().expect("node run");
    assert!(
        out.status.success(),
        "JS re-bind must not SyntaxError: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("12"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// N-2: Stat's documented pack fields are typed — mtime arithmetic infers
/// without an explicit annotation.
#[test]
fn stat_modified_arithmetic_infers_without_annotation() {
    let dir = unique_temp_dir("f62rev_n2");
    let td = dir.join("main.td");
    write_file(
        &td,
        ">>> taida-lang/os => @(Stat)\nst <= Stat[\"/tmp\"]()\nst >=> info\ndiff <= nowMs() - info.modified\nstdout((diff >= 0).toString())\n",
    );
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(
        out.status.success(),
        "Stat field arithmetic must type-check without annotation\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("true"));
    let _ = std::fs::remove_dir_all(&dir);
}
