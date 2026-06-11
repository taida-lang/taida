// F62B-006: a gorilla arm (`| cond |> ><`) is the guide-documented
// unreachable-pattern form — it terminates the program and must not
// participate in cond-arm type unification ([E1603] used to reject it
// because the gorilla literal typed as the empty pack).
//
// F62B-032 (found while fixing): the gorilla exit code is the documented
// fixed exit(1) on every backend — the interpreter used to exit 0 and leak
// the gorilla value into top-level display.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::process::Command;

#[test]
fn gorilla_arm_passes_type_unification() {
    let dir = unique_temp_dir("f62b006_arm");
    let td = dir.join("main.td");
    write_file(
        &td,
        "check x: Int =\n  | x > 0 |> \"ok\"\n  | _ |> ><\n=> :Str\n\nstdout(check(5))\n",
    );
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert!(
        out.status.success(),
        "gorilla arm must type-check\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gorilla_arm_taken_terminates_with_exit_1() {
    let dir = unique_temp_dir("f62b006_taken");
    let td = dir.join("main.td");
    write_file(
        &td,
        "check x: Int =\n  | x > 0 |> \"ok\"\n  | _ |> ><\n=> :Str\n\nstdout(check(-1))\n",
    );
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert_eq!(out.status.code(), Some(1), "gorilla must exit 1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn top_level_gorilla_exits_1_without_display_leak() {
    let dir = unique_temp_dir("f62b032_top");
    let td = dir.join("main.td");
    write_file(&td, "stdout(\"before\")\n><\nstdout(\"after\")\n");
    let out = Command::new(taida_bin()).arg(&td).output().expect("run");
    assert_eq!(
        out.status.code(),
        Some(1),
        "interpreter gorilla must exit 1"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("before"));
    assert!(!stdout.contains("after"));
    assert!(
        !stdout.contains("><"),
        "gorilla value must not leak into display: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
