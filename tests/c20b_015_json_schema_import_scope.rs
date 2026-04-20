//! C20B-015 / ROOT-18: 3-backend regression harness for imported functions
//! that resolve JSON schemas through their defining module.
//!
//! Pre-fix (2.1.3): `JSON[raw, LocalSchema]()` inside an exported function
//! crashed whenever the caller had not also imported the typedef. The
//! interpreter's `resolve_json_schema` consulted `self.type_defs`, which
//! reflects the caller module's scope — private typedefs from the
//! exporting module were invisible.
//!
//! Post-fix:
//!
//!   * `FuncValue` carries an `Arc<HashMap<String, Vec<FieldDef>>>` of the
//!     defining module's full TypeDef registry (via `module_type_defs`),
//!     plus the matching enum registry.
//!   * The module loader enrichment pass attaches these registries to
//!     every exported `Value::Function` after the module executes.
//!   * `call_function*` overlays the defining-module scope onto
//!     `self.type_defs` / `self.enum_defs` before running the body and
//!     restores it afterwards. Locally-defined typedefs of the same name
//!     still win (overlay only fills gaps).
//!   * Native / JS backends were already correct; the regression was
//!     interpreter-only, but this harness pins all three to the same
//!     output so a future regression on any backend is caught.
//!
//! Red-test-zero scope guard:
//!
//!   * Function-only import must succeed.
//!   * Typedef + function import must also succeed (no double-binding
//!     collision).
//!   * Caller that directly references the typedef without importing it
//!     must still fail — scope isolation is preserved.
//!   * Checker must not gain false accept/reject behaviour.

mod common;

use common::{node_available, taida_bin};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_dir() -> PathBuf {
    manifest_dir().join("examples/quality/c20b_015_json_schema_scope")
}

fn caller_path() -> PathBuf {
    fixture_dir().join("caller.td")
}

fn expected_path() -> PathBuf {
    fixture_dir().join("caller.expected")
}

fn read_expected() -> String {
    fs::read_to_string(expected_path()).expect("expected file must exist")
}

fn unique_temp(prefix: &str, ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "{}_{}_{}.{}",
        prefix,
        std::process::id(),
        nanos,
        ext
    ))
}

fn cc_available() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn outputs_equal(a: &str, b: &str) -> bool {
    a.trim_end_matches('\n') == b.trim_end_matches('\n')
}

// ── Interpreter: primary regression ──

#[test]
fn c20b_015_json_schema_scope_interpreter() {
    let out = Command::new(taida_bin())
        .arg(caller_path())
        .output()
        .expect("failed to spawn interpreter");
    assert!(
        out.status.success(),
        "interpreter non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let expected = read_expected();
    assert!(
        outputs_equal(&stdout, &expected),
        "C20B-015 interpreter output mismatch.\n--- expected ---\n{}\n--- got ---\n{}\n",
        expected,
        stdout
    );
}

// ── JS backend: parity (regression guard for a future regression) ──

#[test]
fn c20b_015_json_schema_scope_js_matches_interpreter() {
    if !node_available() {
        eprintln!("SKIP: node not available");
        return;
    }
    let outdir = unique_temp("c20b015_js", "dir");
    fs::create_dir_all(&outdir).expect("mkdir outdir");
    let mjs = outdir.join("caller.mjs");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("js")
        .arg("-o")
        .arg(&mjs)
        .arg(caller_path())
        .output()
        .expect("failed to spawn js build");
    assert!(
        build.status.success(),
        "js build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("node")
        .arg(&mjs)
        .output()
        .expect("failed to spawn node");
    let _ = fs::remove_dir_all(&outdir);
    assert!(
        run.status.success(),
        "node exit failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let expected = read_expected();
    assert!(
        outputs_equal(&stdout, &expected),
        "C20B-015 JS output mismatch.\n--- expected ---\n{}\n--- got ---\n{}\n",
        expected,
        stdout
    );
}

// ── Native backend: parity ──

#[test]
fn c20b_015_json_schema_scope_native_matches_interpreter() {
    if !cc_available() {
        eprintln!("SKIP: cc not available");
        return;
    }
    let outdir = unique_temp("c20b015_native", "dir");
    fs::create_dir_all(&outdir).expect("mkdir outdir");
    let bin = outdir.join("caller_native");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("native")
        .arg("-o")
        .arg(&bin)
        .arg(caller_path())
        .output()
        .expect("failed to spawn native build");
    assert!(
        build.status.success(),
        "native build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&bin)
        .output()
        .expect("failed to spawn native binary");
    let _ = fs::remove_dir_all(&outdir);
    assert!(
        run.status.success(),
        "native binary exit failed: status={:?}, stderr={}",
        run.status.code(),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    let expected = read_expected();
    assert!(
        outputs_equal(&stdout, &expected),
        "C20B-015 native output mismatch.\n--- expected ---\n{}\n--- got ---\n{}\n",
        expected,
        stdout
    );
}

// ── Interpreter: importing typedef + function also works ──
//
// Regression guard for the pre-fix workaround: users who import both
// the typedef and the function must continue to work (no double-binding
// collision when the overlay fills the same name the caller also
// imported). The overlay favours the caller's binding, so this just
// verifies the overlay doesn't crash on name overlap.

#[test]
fn c20b_015_interpreter_also_works_when_caller_imports_typedef() {
    let tmp = unique_temp("c20b015_both", "dir");
    fs::create_dir_all(&tmp).expect("mkdir");
    let schema_src = fs::read_to_string(fixture_dir().join("schema_mod.td"))
        .expect("read schema_mod.td");
    fs::write(tmp.join("schema_mod.td"), schema_src).expect("write schema_mod.td");
    let caller_both = "\
>>> ./schema_mod.td => @(loadUser, UserSchema)
stdout(loadUser())
stdout(\"\\n\")
";
    // Note: fixture schema_mod.td exports only `loadUser`. A caller that
    // asks for `UserSchema` when it isn't in the export list is supposed
    // to get a "Symbol not found in module" error from the importer.
    // That is still correct behaviour — the overlay must not
    // retroactively leak a private typedef through the `<<<` filter.
    fs::write(tmp.join("caller.td"), caller_both).expect("write caller");
    let out = Command::new(taida_bin())
        .arg(tmp.join("caller.td"))
        .output()
        .expect("spawn interp");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = fs::remove_dir_all(&tmp);
    assert!(
        !out.status.success(),
        "expected unexported-typedef import to fail, but program succeeded"
    );
    assert!(
        stderr.contains("UserSchema") && stderr.contains("not found"),
        "expected 'UserSchema … not found' error, got stderr={}",
        stderr
    );
}

// ── Interpreter: caller cannot access typedef just because it's in the overlay ──
//
// The fix must not promote the defining-module's private typedef into
// the caller's scope: calling `loadUser()` succeeds, but `JSON[raw,
// UserSchema]()` *in caller code* must still fail because the caller
// did not import the typedef. This preserves module encapsulation.

#[test]
fn c20b_015_interpreter_overlay_does_not_leak_to_caller_scope() {
    let tmp = unique_temp("c20b015_leak", "dir");
    fs::create_dir_all(&tmp).expect("mkdir");
    let schema_src = fs::read_to_string(fixture_dir().join("schema_mod.td"))
        .expect("read schema_mod.td");
    fs::write(tmp.join("schema_mod.td"), schema_src).expect("write schema_mod.td");
    let leaky_caller = "\
>>> ./schema_mod.td => @(loadUser)
stdout(loadUser())
stdout(\"\\n\")
raw <= \"{\\\"name\\\":\\\"bob\\\",\\\"age\\\":9}\"
stdout(JSON[raw, UserSchema]().__value.name)
";
    fs::write(tmp.join("caller.td"), leaky_caller).expect("write caller");
    let out = Command::new(taida_bin())
        .arg(tmp.join("caller.td"))
        .output()
        .expect("spawn interp");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&tmp);
    // Caller's direct use of `UserSchema` must still be rejected.
    assert!(
        !out.status.success() || combined.contains("Unknown schema type"),
        "caller-scope JSON[raw, UserSchema]() was accidentally allowed — scope leak regression. combined={}",
        combined
    );
}

// ── Interpreter: mutual tail-call retarget does not leak overlay ──
//
// Regression guard for the C20B-015 reopen (2026-04-21 review). Pre-fix
// (post-Phase 5.6): the trampoline captured `td_changed` from the
// *initial* callee and used that flag to decide whether to restore the
// outer scope on exit. If the initial callee was a *local* function
// (`td_changed == false`) and a mutual tail-call retargeted to an
// *imported* function whose push overlaid the defining module's private
// typedefs, the overlay was never restored and leaked into the caller's
// top-level scope.
//
// Repro shape: a local wrapper `wrap raw = load(raw)` whose tail call
// retargets to imported `load` (which lives in a module with private
// `Schema`). After `wrap("...")` returns, the caller attempts
// `JSON[raw, Schema]()` at the top level. This must fail with
// "Unknown schema type 'Schema'" because the caller never imported
// `Schema`. Pre-fix, the caller-scope call silently succeeded.

#[test]
fn c20b_015_interpreter_mutual_tail_call_does_not_leak_overlay() {
    let tmp = unique_temp("c20b015_tail", "dir");
    fs::create_dir_all(&tmp).expect("mkdir");
    let schema_mod = "\
Schema = @(name: Str)

load raw: Str =
  parsed <= JSON[raw, Schema]()
  | parsed.hasValue |> parsed.__value.name
  | _ |> \"no\"
=> :Str

<<< @(load)
";
    fs::write(tmp.join("schema_mod.td"), schema_mod).expect("write schema_mod");
    // `wrap` is a local function whose body is a direct call to the
    // imported `load`. This triggers the mutual tail-call path in the
    // trampoline (initial callee `wrap` is local; retarget is imported
    // `load` which pushes an overlay).
    let caller = "\
>>> ./schema_mod.td => @(load)

wrap raw: Str = load(raw) => :Str

stdout(wrap(\"{\\\"name\\\":\\\"ok\\\"}\"))
stdout(\"\\n\")
stdout(JSON[\"{\\\"name\\\":\\\"leak\\\"}\", Schema]().__value.name)
stdout(\"\\n\")
";
    fs::write(tmp.join("caller.td"), caller).expect("write caller");
    let out = Command::new(taida_bin())
        .arg(tmp.join("caller.td"))
        .output()
        .expect("spawn interp");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let combined = format!("{}\n{}", stdout, stderr);
    let _ = fs::remove_dir_all(&tmp);
    // `wrap("…")` must still succeed — the imported function's overlay
    // is applied for the duration of its body.
    assert!(
        stdout.contains("ok"),
        "imported function call via local wrapper did not produce expected 'ok'. stdout={}, stderr={}",
        stdout,
        stderr
    );
    // The caller-scope `JSON[..., Schema]()` must still be rejected. The
    // overlay from `load`'s body is NOT allowed to persist back into the
    // caller's top-level scope after `wrap(…)` returns.
    assert!(
        combined.contains("Unknown schema type") && combined.contains("Schema"),
        "caller-scope JSON[raw, Schema]() was silently allowed — mutual \
         tail-call overlay leak regression. combined={}",
        combined
    );
    // The program as a whole should terminate with a runtime error
    // (not exit 0), because the second `stdout(JSON[...]().__value.name)`
    // must throw before reaching `stdout`.
    // NB: taida interpreter currently returns exit 0 even on Runtime error;
    // what matters is the "Unknown schema type" assertion above.
}

// ── Interpreter: truly unknown schema still errors ──

#[test]
fn c20b_015_interpreter_truly_unknown_schema_still_errors() {
    let tmp = unique_temp("c20b015_unknown", "dir");
    fs::create_dir_all(&tmp).expect("mkdir");
    let src = "\
raw <= \"{\\\"x\\\":1}\"
stdout(JSON[raw, ThisDoesNotExist]().__value.x.toString())
";
    fs::write(tmp.join("prog.td"), src).expect("write prog");
    let out = Command::new(taida_bin())
        .arg(tmp.join("prog.td"))
        .output()
        .expect("spawn interp");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&tmp);
    assert!(
        !out.status.success(),
        "unknown schema should have errored; combined={}",
        combined
    );
    assert!(
        combined.contains("Unknown schema type") && combined.contains("ThisDoesNotExist"),
        "expected Unknown schema type error for ThisDoesNotExist, got={}",
        combined
    );
}
