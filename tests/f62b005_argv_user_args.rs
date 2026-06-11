// F62B-005: argv() exposes only the user's arguments.
//
// The docs contract (os.md §4.3): everything after the first standalone
// `--`; taida's own options never leak. Previously the interpreter sliced
// the raw process argv positionally, so `--` itself, `--no-check`, and even
// the script name leaked into argv() depending on option order.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::path::PathBuf;
use std::process::Command;

const PROGRAM: &str = ">>> taida-lang/os => @(argv)\nargs <= argv()\nstdout(args.toString())\n";

fn setup(label: &str) -> (PathBuf, PathBuf) {
    let dir = unique_temp_dir(label);
    let td = dir.join("main.td");
    write_file(&td, PROGRAM);
    (dir, td)
}

fn run_with(td: &PathBuf, args: &[&str]) -> String {
    let out = Command::new(taida_bin())
        .arg(td)
        .args(args)
        .output()
        .expect("run taida");
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

#[test]
fn dashdash_separator_is_not_included() {
    let (dir, td) = setup("f62b005_sep");
    assert_eq!(
        run_with(&td, &["--", "version", "--x"]),
        r#"@["version", "--x"]"#
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn taida_options_do_not_leak() {
    let (dir, td) = setup("f62b005_opts");
    // --no-check after the script, before `--`: taida's own option.
    let out = Command::new(taida_bin())
        .arg(&td)
        .args(["--no-check", "--", "version"])
        .output()
        .expect("run");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        r#"@["version"]"#
    );
    // --no-check before the script: script name must not leak either.
    let out2 = Command::new(taida_bin())
        .args(["--no-check"])
        .arg(&td)
        .args(["--", "version"])
        .output()
        .expect("run");
    assert_eq!(
        String::from_utf8_lossy(&out2.stdout).trim_end(),
        r#"@["version"]"#
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn args_after_dashdash_are_verbatim_even_if_they_look_like_options() {
    let (dir, td) = setup("f62b005_verbatim");
    assert_eq!(run_with(&td, &["--", "--no-check"]), r#"@["--no-check"]"#);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bare_args_without_separator_still_work() {
    let (dir, td) = setup("f62b005_bare");
    assert_eq!(run_with(&td, &["a", "b"]), r#"@["a", "b"]"#);
    assert_eq!(run_with(&td, &[]), "@[]");
    let _ = std::fs::remove_dir_all(&dir);
}
