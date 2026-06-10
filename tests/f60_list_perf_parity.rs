/// Cross-backend pins for the list-performance track's correctness
/// findings.
mod common;

use common::{run_interpreter, taida_bin, unique_temp_dir, wasmtime_bin};
use std::path::Path;
use std::process::Command;

fn build_and_run_native(td: &Path, dir: &Path, stem: &str) -> String {
    let bin = dir.join(format!("{stem}_native"));
    let status = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("taida build native runs");
    assert!(status.success(), "native build failed for {stem}");
    let out = Command::new(&bin).output().expect("native binary runs");
    assert!(out.status.success(), "native run failed for {stem}");
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn build_and_run_wasm(td: &Path, dir: &Path, stem: &str) -> Option<String> {
    let wasmtime = wasmtime_bin()?;
    let wasm = dir.join(format!("{stem}.wasm"));
    let status = Command::new(taida_bin())
        .args(["build", "wasm-min"])
        .arg(td)
        .arg("-o")
        .arg(&wasm)
        .status()
        .expect("taida build wasm-min runs");
    assert!(status.success(), "wasm build failed for {stem}");
    let out = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("wasmtime runs");
    assert!(out.status.success(), "wasm run failed for {stem}");
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn assert_parity(dir: &Path, stem: &str, source: &str) -> String {
    let td = dir.join(format!("{stem}.td"));
    std::fs::write(&td, source).expect("write fixture");
    let interp = run_interpreter(&td).expect("interpreter runs");
    let native = build_and_run_native(&td, dir, stem);
    assert_eq!(interp, native, "{stem}: interp vs native");
    if let Some(wasm) = build_and_run_wasm(&td, dir, stem) {
        assert_eq!(interp, wasm, "{stem}: interp vs wasm-min");
    } else {
        eprintln!("SKIP: wasmtime not found, wasm leg skipped for {stem}");
    }
    interp
}

/// A function whose body ends in an unmolded Fold could not be written
/// at all: the static return-type table read the LIST argument instead
/// of the INIT accumulator, so `=> :Int` failed with "body returns
/// @[Int]" on every such function.
#[test]
fn function_ending_in_fold_typechecks_and_runs() {
    let dir = unique_temp_dir("f60_fold_ret");
    let out = assert_parity(
        &dir,
        "fold_ret",
        r#"sumIt u: Int =
  xs <= @[1, 2, 3]
  Fold[xs, 0, _ acc: Int x: Int = acc + x]() >=> t
  t
=> :Int

joinIt u: Int =
  ws <= @["a", "b"]
  Foldr[ws, "", _ acc: Str w: Str = acc + w]() >=> s
  s.length()
=> :Int

stdout(sumIt(1))
stdout(joinIt(1))
"#,
    );
    assert_eq!(out, "6\n2");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The consume-append rewrite: a tail-recursive `f(n-1, Append[acc, x]())`
/// pushes in place once the loop owns its accumulator. The FIRST
/// activation must NOT consume — the accumulator may be the caller's
/// list. This is the pin for that detach.
#[test]
fn consume_append_detaches_callers_list() {
    let dir = unique_temp_dir("f60_consume_detach");
    let out = assert_parity(
        &dir,
        "consume_detach",
        r#"build n: Int acc: @[Int] =
  | n == 0 |> acc
  | _ |> build(n - 1, Append[acc, n]())
=> :@[Int]

xs <= @[100]
ys <= build(3, xs)
stdout(xs)
stdout(ys)
zs <= build(2, xs)
stdout(xs)
stdout(zs)
"#,
    );
    assert_eq!(out, "@[100]\n@[100, 3, 2, 1]\n@[100]\n@[100, 2, 1]");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Fail-closed shapes: anything that could observe the old list keeps
/// the copy semantics on every backend.
#[test]
fn consume_append_fails_closed_on_unsafe_shapes() {
    let dir = unique_temp_dir("f60_consume_failclosed");
    // acc used twice in the recursive arm (second use after the append).
    let out = assert_parity(
        &dir,
        "twice",
        r#"build n: Int acc: @[Int] =
  | n == 0 |> acc
  | _ |>
    grown <= Append[acc, n]()
    last <= acc.length()
    build(n - 1, Append[grown, last]())
=> :@[Int]

stdout(build(2, @[7]).length())
"#,
    );
    assert_eq!(out, "5");
    // Append[p, p-derived] — the item reads the accumulator.
    let out2 = assert_parity(
        &dir,
        "self_item",
        r#"build n: Int acc: @[Int] =
  | n == 0 |> acc
  | _ |> build(n - 1, Append[acc, acc.length()]())
=> :@[Int]

stdout(build(3, @[]))
"#,
    );
    assert_eq!(out2, "@[0, 1, 2]");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Large sequential builds complete on wasm (the O(n^2) copies used to
/// exhaust the 2GB boxed-value address space on the second call) and
/// agree across backends.
#[test]
fn large_sequential_build_completes_everywhere() {
    let dir = unique_temp_dir("f60_consume_large");
    let out = assert_parity(
        &dir,
        "large",
        r#"build n: Int acc: @[Int] =
  | n == 0 |> acc
  | _ |> build(n - 1, Append[acc, n]())
=> :@[Int]

p u: Int =
  nums <= build(10000, @[])
  nums.length()
=> :Int

stdout(p(1) + p(2))
"#,
    );
    assert_eq!(out, "20000");
    let _ = std::fs::remove_dir_all(&dir);
}
