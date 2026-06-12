// F62B-008 + F62B-016: types and function signatures must cross the module
// boundary on every pipeline.
//
// - F62B-008: an imported pack type used as a `JSON[raw, Schema]()` schema
//   was rejected with [E1541] even though the message says "import it";
//   deeper chains slipped past the checker and died at runtime.
// - F62B-016: the codegen driver's pre-lowering checker was constructed
//   without the source path, so imported types / signatures never
//   registered there — imported-call results typed `?` in the Typed HIR
//   and the build failed on the residual-unknown gate (native and wasm).
//
// Fixes: register_imported_types now registers imported ClassLikeDefs
// (BuchiPack + inheritance) like local declarations, and all three driver
// checker sites wire the source path.

mod common;

use common::{taida_bin, unique_temp_dir, write_file};
use std::path::PathBuf;
use std::process::Command;

fn run_interp(td: &PathBuf, dir: &PathBuf) -> std::process::Output {
    Command::new(taida_bin())
        .arg(td)
        .current_dir(dir)
        .output()
        .expect("run taida")
}

fn build_and_run_native(dir: &PathBuf, td: &PathBuf) -> Result<std::process::Output, String> {
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&bin)
        .current_dir(dir)
        .output()
        .expect("native build");
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).into_owned());
    }
    Ok(Command::new(&bin).output().expect("run binary"))
}

/// F62B-008 symptom 1: a directly imported pack type works as a JSON schema.
#[test]
fn imported_pack_type_works_as_json_schema() {
    let dir = unique_temp_dir("f62b008_direct");
    write_file(
        &dir.join("mod.td"),
        "Point = @(x: Int, y: Int)\n<<< @(Point)\n",
    );
    let td = dir.join("main.td");
    write_file(
        &td,
        ">>> ./mod.td => @(Point)\np <= JSON['{\"x\": 7, \"y\": 2}', Point]()\np >=> pt\nstdout(pt.x.toString())\n",
    );
    let out = run_interp(&td, &dir);
    assert!(
        out.status.success(),
        "imported schema must check and run\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains('7'));
    let _ = std::fs::remove_dir_all(&dir);
}

/// F62B-008 symptom 2: a schema imported two modules deep resolves at
/// check time AND at runtime, on the interpreter and the native build.
#[test]
fn deep_import_chain_schema_resolves_everywhere() {
    let dir = unique_temp_dir("f62b008_deep");
    write_file(
        &dir.join("types.td"),
        "MsgPayload = @(kind: Str, body: Str)\n<<< @(MsgPayload)\n",
    );
    write_file(
        &dir.join("daemon.td"),
        ">>> ./types.td => @(MsgPayload)\ndecode raw: Str =\n  p <= JSON[raw, MsgPayload]()\n  p >=> payload\n  payload.body\n=> :Str\n<<< @(decode)\n",
    );
    write_file(
        &dir.join("commands.td"),
        ">>> ./daemon.td => @(decode)\nhandle raw: Str =\n  decode(raw)\n=> :Str\n<<< @(handle)\n",
    );
    let td = dir.join("main.td");
    write_file(
        &td,
        ">>> ./commands.td => @(handle)\nstdout(handle('{\"kind\": \"x\", \"body\": \"deep-ok\"}'))\n",
    );
    let interp = run_interp(&td, &dir);
    assert!(
        interp.status.success(),
        "deep chain must run on interp\nstderr={}",
        String::from_utf8_lossy(&interp.stderr)
    );
    assert!(String::from_utf8_lossy(&interp.stdout).contains("deep-ok"));

    let native = build_and_run_native(&dir, &td).expect("native must compile");
    assert!(native.status.success());
    assert!(String::from_utf8_lossy(&native.stdout).contains("deep-ok"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// F62B-016: an imported function's return type reaches call sites inside
/// pack literals — the wasm build used to fail on `value: ?` at the
/// residual-unknown gate.
#[test]
fn imported_call_result_in_pack_field_compiles_on_wasm() {
    let wasmtime = match common::wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: wasmtime unavailable");
            return;
        }
    };
    let dir = unique_temp_dir("f62b016_wasm");
    write_file(
        &dir.join("mod.td"),
        "headerValueOf k: Str =\n  `v-${k}`\n=> :Str\n<<< @(headerValueOf)\n",
    );
    let td = dir.join("main.td");
    write_file(
        &td,
        ">>> ./mod.td => @(headerValueOf)\npack <= @(name <= \"ct\", value <= headerValueOf(\"a\"))\nstdout(pack.value)\n",
    );
    let wasm = dir.join("main.wasm");
    let build = Command::new(taida_bin())
        .args(["build", "wasm-wasi"])
        .arg(&td)
        .arg("-o")
        .arg(&wasm)
        .current_dir(&dir)
        .output()
        .expect("wasm build");
    assert!(
        build.status.success(),
        "wasm build must not hit the residual-unknown gate\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&wasmtime)
        .args(["run", "--"])
        .arg(&wasm)
        .output()
        .expect("wasmtime run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("v-a"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Imported error types register through the inheritance path.
#[test]
fn imported_error_type_registers() {
    let dir = unique_temp_dir("f62b008_error");
    write_file(
        &dir.join("mod.td"),
        "Error => AppError = @(code: Int)\n<<< @(AppError)\n",
    );
    let td = dir.join("main.td");
    write_file(
        &td,
        ">>> ./mod.td => @(AppError)\ne <= AppError(message <= \"boom\", code <= 7)\nstdout(e.code.toString())\n",
    );
    let out = run_interp(&td, &dir);
    assert!(
        out.status.success(),
        "imported error type must check and run\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains('7'));
    let _ = std::fs::remove_dir_all(&dir);
}
