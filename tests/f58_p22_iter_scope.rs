/// Regression tests for the iteration-scope arena watermark.
///
/// Tail-recursive loops whose iteration-local allocations provably cannot
/// escape get an arena watermark: enter at function entry, reset right
/// before each loop back-edge, exit before return. The reset rewinds the
/// bump arena, so a million-iteration mold loop no longer accumulates
/// hundreds of megabytes — but every value that crosses an iteration
/// boundary must still be correct, the freelists must never hold a
/// rewound object, and loops that fail the safety conditions (enum
/// descriptor registration, non-scalar parameters) must be left alone.
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
    assert_parity_with(dir, stem, source, true)
}

fn assert_parity_with(dir: &Path, stem: &str, source: &str, wasm_leg: bool) -> String {
    let td = dir.join(format!("{stem}.td"));
    std::fs::write(&td, source).expect("write fixture");
    let interp = run_interpreter(&td).expect("interpreter runs");
    let native = build_and_run_native(&td, dir, stem);
    assert_eq!(interp, native, "{stem}: interp vs native");
    if !wasm_leg {
        return interp;
    }
    if let Some(wasm) = build_and_run_wasm(&td, dir, stem) {
        assert_eq!(interp, wasm, "{stem}: interp vs wasm-min");
    } else {
        eprintln!("SKIP: wasmtime not found, wasm leg skipped for {stem}");
    }
    interp
}

/// The canonical rewind target: two mold allocations per iteration, deep
/// enough that the arena would otherwise span dozens of chunks. Values
/// must survive the per-iteration rewind exactly.
#[test]
fn mold_loop_values_survive_iteration_rewind() {
    let dir = unique_temp_dir("f58_p22_mold_loop");
    let out = assert_parity(
        &dir,
        "mold_loop",
        r#"laxLoop n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    Lax[n]() >=> val
    Div[val, 3]() >=> divided
    laxLoop(n - 1, acc + divided)
=> :Int
stdout(laxLoop(200000, 0))
"#,
    );
    assert_eq!(out, "6666633333");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Strings allocated inside the iteration (concat + length) must be fully
/// consumed before the rewind and never resurface through the freelists.
#[test]
fn string_loop_values_survive_iteration_rewind() {
    let dir = unique_temp_dir("f58_p22_str_loop");
    let out = assert_parity_with(
        &dir,
        "str_loop",
        r#"strLoop n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    s <= "ab" + "cdef"
    strLoop(n - 1, acc + s.length())
=> :Int
stdout(strLoop(50000, 0))
"#,
        true,
    );
    assert_eq!(out, "300000");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pack construction + field reads per iteration (the buchipack shape).
#[test]
fn pack_loop_values_survive_iteration_rewind() {
    let dir = unique_temp_dir("f58_p22_pack_loop");
    let out = assert_parity(
        &dir,
        "pack_loop",
        r#"accessLoop n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    pack <= @(x <= n, y <= n * 2, z <= n * 3)
    sum <= pack.x + pack.y + pack.z
    accessLoop(n - 1, acc + sum)
=> :Int
stdout(accessLoop(100000, 0))
"#,
    );
    assert_eq!(out, "30000300000");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A loop that builds packs with an Enum-typed field registers the pack
/// pointer in the enum-descriptor registry — that call disqualifies the
/// loop from the watermark (a rewind would systematically recycle the
/// registered address). The loop must still run correctly, and the enum
/// must keep its variant-name JSON shape afterwards.
#[test]
fn enum_field_pack_loop_is_exempt_and_correct() {
    let dir = unique_temp_dir("f58_p22_enum_loop");
    // wasm leg skipped: wasm-min rejects taida_register_field_enum at
    // compile time (pre-existing profile limitation, unrelated to the
    // watermark).
    let out = assert_parity_with(
        &dir,
        "enum_loop",
        r#"Enum => Phase = :Solid :Liquid :Gas

enumLoop n: Int acc: Int =
  | n == 0 |> acc
  | _ |>
    rec <= @(phase <= Phase:Liquid(), step <= n)
    enumLoop(n - 1, acc + rec.step)
=> :Int
stdout(enumLoop(2000, 0))
last <= @(phase <= Phase:Gas(), step <= 7)
stdout(jsonEncode(last))
"#,
        false,
    );
    assert_eq!(out, "2001000\n{\"phase\":\"Gas\",\"step\":7}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A throw escapes the iteration scope via longjmp without running the
/// exit hook; the error ceiling must observe the error normally and
/// later allocations / freelist recycling must keep working.
#[test]
fn throw_through_iteration_scope_recovers() {
    let dir = unique_temp_dir("f58_p22_throw");
    let out = assert_parity(
        &dir,
        "throw_loop",
        r#"Error => LoopError = @(message: Str)

boom n: Int acc: Int =
  | n == 5 |> LoopError(type <= "LoopError", message <= "boom").throw()
  | n == 0 |> acc
  | _ |>
    s <= "ab" + "cd"
    boom(n - 1, acc + s.length())
=> :Int

safeRun input: Int =
  |== error: Error =
    0 - 1
  => :Int
  boom(input, 0)
=> :Int

stdout(safeRun(20))
stdout(safeRun(4))
after <= "still" + " alive"
stdout(after)
"#,
    );
    assert_eq!(out, "-1\n16\nstill alive");
    let _ = std::fs::remove_dir_all(&dir);
}
