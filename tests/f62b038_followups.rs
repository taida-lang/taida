// F62B-038: final-review dormant guards — the runtime defences behind the
// E1517/E1511 static rejections plus the transitive schema-passing closure,
// pinned across backends.
//
//   #6  — InCage/Uncage with a non-builder first argument (reachable when an
//         Unknown-typed value bypasses E1517; simulated here with
//         `--no-check`) reports and terminates identically on interp /
//         native / wasm: "Runtime error: <mold> requires a CageBuilder as
//         its first argument (start the chain with `Cage[subject]()`), got
//         <Type>", exit 1. Native used to walk the value as a raw pack
//         pointer (segfault) or abort without the type name; wasm used to
//         poison the builder silently and only surface at Uncage as an
//         Async rejection.
//   #9  — non-generic `fn[1](2)` on the unchecked-eval path is a runtime
//         error: the bracket is the legacy positional call, so silently
//         dropping the bracket values regressed the old defence (the
//         checker rejects the form statically as E1511).
//   #11 — generic→generic schema forwarding (`outer[T] = inner[T](..)`)
//         checks and lowers through the transitive hidden-schema closure;
//         the builder's stdout display never leaks a raw pointer.

mod common;

use common::{taida_bin, unique_temp_dir, wasmtime_bin, write_file};
use std::fs;
use std::process::{Command, Output};
use taida::interpreter::{HostCallMockStep, Interpreter, Value};

fn run_interp_no_check(label: &str, source: &str) -> Output {
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let output = Command::new(taida_bin())
        .arg("--no-check")
        .arg(&src)
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    output
}

/// Build natively (optionally bypassing the checker) and run the binary.
fn build_and_run_native(label: &str, source: &str, no_check: bool) -> Output {
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let bin = dir.join("main_bin");
    let mut build = Command::new(taida_bin());
    build.arg("build");
    if no_check {
        build.arg("--no-check");
    }
    let build = build
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
    run
}

/// Build for wasm-wasi (optionally bypassing the checker) and run under
/// wasmtime. Returns `None` when wasmtime is not installed.
fn build_and_run_wasm(label: &str, source: &str, no_check: bool) -> Option<Output> {
    let wasmtime = wasmtime_bin()?;
    let dir = unique_temp_dir(label);
    let src = dir.join("main.td");
    write_file(&src, source);
    let wasm = dir.join("main.wasm");
    let mut build = Command::new(taida_bin());
    build.arg("build").arg("wasm-wasi");
    if no_check {
        build.arg("--no-check");
    }
    let build = build
        .arg(&src)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("wasm build");
    assert!(
        build.status.success(),
        "wasm build must succeed\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("run wasmtime");
    let _ = fs::remove_dir_all(&dir);
    Some(run)
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ── #6: non-builder chain input — backend-symmetric failure ──────

const NON_BUILDER_INCAGE: &str = "x <= 5\nInCage[x, \"get\", @[]]()\n";
const NON_BUILDER_UNCAGE: &str = "s <= \"hello\"\nUncage[s, \"all\", Str]() >=> out\nstdout(out)\n";

const INCAGE_GOT_INT: &str = "Runtime error: InCage requires a CageBuilder as its \
                              first argument (start the chain with `Cage[subject]()`), got Int";
const UNCAGE_GOT_STR: &str = "Runtime error: Uncage requires a CageBuilder as its \
                              first argument (start the chain with `Cage[subject]()`), got Str";

/// interp: the reference behaviour — message + exit 1.
#[test]
fn incage_non_builder_reports_and_exits_on_interp() {
    let out = run_interp_no_check("f62b038_incage_interp", NON_BUILDER_INCAGE);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_text(&out).contains(INCAGE_GOT_INT),
        "expected the typed builder rejection, got: {}",
        stderr_text(&out)
    );
}

/// native: same message, same exit code — and a real exit, not a signal
/// (`code()` is `None` on SIGSEGV/SIGABRT, which is what the unguarded
/// pack walk / runtime abort produced before).
#[test]
fn incage_non_builder_reports_and_exits_on_native() {
    let out = build_and_run_native("f62b038_incage_native", NON_BUILDER_INCAGE, true);
    assert_eq!(
        out.status.code(),
        Some(1),
        "must exit(1), not die on a signal; stderr={}",
        stderr_text(&out)
    );
    assert!(
        stderr_text(&out).contains(INCAGE_GOT_INT),
        "expected the typed builder rejection, got: {}",
        stderr_text(&out)
    );
}

/// wasm: the InCage failure is reported immediately (the old behaviour
/// poisoned the builder silently and only failed at Uncage, as an Async
/// rejection rather than a report).
#[test]
fn incage_non_builder_reports_and_exits_on_wasm() {
    let Some(out) = build_and_run_wasm("f62b038_incage_wasm", NON_BUILDER_INCAGE, true) else {
        eprintln!("wasmtime not installed; skipping");
        return;
    };
    assert_eq!(
        out.status.code(),
        Some(1),
        "must exit(1) via proc_exit, not trap; stderr={}",
        stderr_text(&out)
    );
    assert!(
        stderr_text(&out).contains(INCAGE_GOT_INT),
        "expected the typed builder rejection, got: {}",
        stderr_text(&out)
    );
}

/// Uncage names its own mold and the actual type (Str here) — all three
/// backends produce the identical report.
#[test]
fn uncage_non_builder_reports_got_str_on_all_backends() {
    let interp = run_interp_no_check("f62b038_uncage_interp", NON_BUILDER_UNCAGE);
    assert_eq!(interp.status.code(), Some(1));
    assert!(
        stderr_text(&interp).contains(UNCAGE_GOT_STR),
        "interp: expected the typed builder rejection, got: {}",
        stderr_text(&interp)
    );

    let native = build_and_run_native("f62b038_uncage_native", NON_BUILDER_UNCAGE, true);
    assert_eq!(native.status.code(), Some(1));
    assert!(
        stderr_text(&native).contains(UNCAGE_GOT_STR),
        "native: expected the typed builder rejection, got: {}",
        stderr_text(&native)
    );

    if let Some(wasm) = build_and_run_wasm("f62b038_uncage_wasm", NON_BUILDER_UNCAGE, true) {
        assert_eq!(
            wasm.status.code(),
            Some(1),
            "wasm: must exit(1) via proc_exit, not trap; stderr={}",
            stderr_text(&wasm)
        );
        assert!(
            stderr_text(&wasm).contains(UNCAGE_GOT_STR),
            "wasm: expected the typed builder rejection, got: {}",
            stderr_text(&wasm)
        );
    } else {
        eprintln!("wasmtime not installed; skipping the wasm leg");
    }
}

// ── #9: non-generic bracket + call arguments (unchecked eval) ────

/// `f[7](3)` must be a runtime error on the unchecked path — silently
/// dropping the bracket and running `f(3)` is the regressed behaviour.
#[test]
fn non_generic_bracket_with_call_args_is_a_runtime_error_unchecked() {
    let out = run_interp_no_check(
        "f62b038_bracket_args",
        "f x: Int = x => :Int\nstdout(f[7](3))\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_text(&out).contains("cannot take both bracket values and call arguments"),
        "expected the combined-form rejection, got: {}",
        stderr_text(&out)
    );
    assert!(
        !stdout_text(&out).contains('3'),
        "the call must not silently run as f(3); stdout={}",
        stdout_text(&out)
    );
}

/// The legacy positional bracket call `f[3]()` keeps working unchecked.
#[test]
fn non_generic_legacy_bracket_call_still_works_unchecked() {
    let out = run_interp_no_check(
        "f62b038_bracket_legacy",
        "f x: Int = x => :Int\nstdout(f[3]())\n",
    );
    assert!(
        out.status.success(),
        "legacy bracket call must run\nstderr={}",
        stderr_text(&out)
    );
    assert_eq!(stdout_text(&out), "3\n");
}

// ── #11: generic→generic schema forwarding ───────────────────────

const G2G_HELPER: &str = r#"inner[T] db: CageBuilder  sql: Str =
  db => InCage[_, "prepare", @[sql]]() => Uncage[_, "all", T]() >=> rows
  rows
=> :T

outer[T] db: CageBuilder  sql: Str =
  inner[T](db, sql) => r
  r
=> :T

cap <= HostCapability["CAP", "mock/kind"]()
base <= Cage[cap]()
out <= outer[Str](base, "select 1")
out
"#;

fn g2g_fixture() -> Vec<HostCallMockStep> {
    vec![
        HostCallMockStep {
            method: "prepare".to_string(),
            args: vec![Value::str("select 1".to_string())],
            result: Value::str("stmt".to_string()),
        },
        HostCallMockStep {
            method: "all".to_string(),
            args: Vec::new(),
            result: Value::str("row".to_string()),
        },
    ]
}

/// interp: the forwarded schema resolves end-to-end through the fixture.
#[test]
fn generic_to_generic_forwarding_resolves_in_interp() {
    let (program, parse_errors) = taida::parser::parse(G2G_HELPER);
    assert!(parse_errors.is_empty(), "parse errors: {:?}", parse_errors);
    let mut interpreter = Interpreter::new();
    interpreter.set_host_capability_mock_steps("CAP", g2g_fixture());
    let result = interpreter
        .eval_program(&program)
        .expect("host capability fixture should evaluate");
    assert_eq!(result, Value::str("row".to_string()));
}

/// native: the transitive closure gives `outer` the hidden schema param,
/// so the program lowers (it used to fail with a non-diagnostic "Unknown
/// schema type 'T'") and reaches the deterministic session-less rejection.
#[test]
fn generic_to_generic_forwarding_lowers_natively() {
    let source = format!("{G2G_HELPER}stdout(out)\n");
    let run = build_and_run_native("f62b038_g2g_native", &source, false);
    assert!(!run.status.success());
    assert!(
        stderr_text(&run).contains("host capabilities are not available"),
        "expected the session-less rejection, got: {}",
        stderr_text(&run)
    );
}

/// checker: `outer` itself becomes schema-passing through the closure, so
/// its inference-form call is rejected with the explicit-call guidance
/// (it used to pass the checker and die inside native lowering).
#[test]
fn generic_to_generic_forwarding_requires_explicit_calls() {
    let dir = unique_temp_dir("f62b038_g2g_inference");
    let src = dir.join("main.td");
    write_file(
        &src,
        &G2G_HELPER.replace(
            "outer[Str](base, \"select 1\")",
            "outer(base, \"select 1\")",
        ),
    );
    let output = Command::new(taida_bin())
        .arg(&src)
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success());
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1510]") && stderr.contains("'outer'"),
        "expected the explicit-arguments requirement on the forwarding generic, got: {stderr}"
    );
}

// ── #11: builder display stability (tag-fix pin) ─────────────────

const BUILDER_DISPLAY: &str = concat!(
    "db <= HostCapability[\"DB\", \"mock/kind\"]()\n",
    "Cage[db]() => InCage[_, \"get\", @[\"k\"]]() => b\n",
    "stdout(b)\n",
);

/// Longest run of consecutive ASCII digits in `s` — a pointer-sized
/// number in the output is the regression signature (the untagged
/// subject used to render as a raw pointer value).
fn longest_digit_run(s: &str) -> usize {
    let mut best = 0usize;
    let mut cur = 0usize;
    for c in s.chars() {
        if c.is_ascii_digit() {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Displaying a builder is stable on native and wasm: exits 0 and never
/// leaks a raw pointer for the tagged subject / steps.
///
/// NOTE: the native/wasm display is `@()` while the interpreter prints
/// the full field form — pack display parity for runtime-internal `__`
/// fields is a separate pre-existing gap (field names are not registered
/// with the runtime and `__` fields are display-skipped), tracked as its
/// own blocker; this pin covers the #2 tag fix only (no raw pointers, no
/// crash).
#[test]
fn builder_display_is_stable_and_pointer_free() {
    let native = build_and_run_native("f62b038_bdisplay_native", BUILDER_DISPLAY, false);
    assert!(
        native.status.success(),
        "displaying a builder must not crash natively\nstderr={}",
        stderr_text(&native)
    );
    assert!(
        longest_digit_run(&stdout_text(&native)) < 6,
        "native builder display must not leak a raw pointer, got: {}",
        stdout_text(&native)
    );

    if let Some(wasm) = build_and_run_wasm("f62b038_bdisplay_wasm", BUILDER_DISPLAY, false) {
        assert!(
            wasm.status.success(),
            "displaying a builder must not crash on wasm\nstderr={}",
            stderr_text(&wasm)
        );
        assert!(
            longest_digit_run(&stdout_text(&wasm)) < 6,
            "wasm builder display must not leak a raw pointer, got: {}",
            stdout_text(&wasm)
        );
    } else {
        eprintln!("wasmtime not installed; skipping the wasm leg");
    }
}

// ── F62B-040: schema-passing across module boundaries ────────────
//
// Each module is lowered by its own `Lowering` instance, so an imported
// generic's hidden-schema metadata cannot be observed from the exporting
// module's pass. Before the fix, explicit calls to an imported
// schema-passing generic emitted no hidden schema arguments (an ABI
// arity mismatch — wasm's C type-check rejected the build outright,
// native silently called a 3-arg function with 2 arguments), inferable
// inference-form calls passed the checker entirely, and a forwarding
// generic whose callee lives in another file was not detected. The
// checker now computes a recursive per-module closure at import
// registration and hands the metadata to lowering next to the typed-HIR
// table. A successful wasm build doubles as a structural arity proof.

const XMOD_LIB: &str = r#"queryAll[T] db: CageBuilder  sql: Str =
  db => InCage[_, "prepare", @[sql]]() => Uncage[_, "all", T]() >=> rows
  rows
=> :T

<<< @(queryAll)
"#;

/// Write a multi-file module fixture and build/run the entry natively.
fn build_and_run_native_modules(label: &str, files: &[(&str, &str)], entry: &str) -> Output {
    let dir = unique_temp_dir(label);
    for (name, content) in files {
        write_file(&dir.join(name), content);
    }
    let bin = dir.join("main_bin");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg(dir.join(entry))
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
    run
}

/// Write a multi-file module fixture and build/run the entry on wasm.
/// Returns `None` when wasmtime is not installed.
fn build_and_run_wasm_modules(label: &str, files: &[(&str, &str)], entry: &str) -> Option<Output> {
    let wasmtime = wasmtime_bin()?;
    let dir = unique_temp_dir(label);
    for (name, content) in files {
        write_file(&dir.join(name), content);
    }
    let wasm = dir.join("main.wasm");
    let build = Command::new(taida_bin())
        .arg("build")
        .arg("wasm-wasi")
        .arg(dir.join(entry))
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("wasm build");
    assert!(
        build.status.success(),
        "wasm build must succeed (a failure here is the hidden-schema \
         ABI arity mismatch)\nstderr={}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("run wasmtime");
    let _ = fs::remove_dir_all(&dir);
    Some(run)
}

/// Explicit call to an imported schema-passing generic: lowers on native
/// and wasm with the hidden schema argument attached, and reaches the
/// deterministic session-less rejection.
#[test]
fn imported_schema_passing_explicit_call_lowers() {
    let main = concat!(
        ">>> ./lib.td => @(queryAll)\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= queryAll[Str](base, \"select 1\")\n",
        "stdout(out)\n",
    );
    let files = [("lib.td", XMOD_LIB), ("main.td", main)];
    let native = build_and_run_native_modules("f62b040_explicit_native", &files, "main.td");
    assert!(!native.status.success());
    assert!(
        stderr_text(&native).contains("host capabilities are not available"),
        "native: expected the session-less rejection, got: {}",
        stderr_text(&native)
    );
    if let Some(wasm) = build_and_run_wasm_modules("f62b040_explicit_wasm", &files, "main.td") {
        assert!(!wasm.status.success());
        assert!(
            stderr_text(&wasm).contains("HostCapabilityError"),
            "wasm: expected the host-capability rejection, got: {}",
            stderr_text(&wasm)
        );
    } else {
        eprintln!("wasmtime not installed; skipping the wasm leg");
    }
}

/// The alias form keeps the metadata: `queryAll => qa` calls as
/// `qa[Str](...)` with the schema argument attached.
#[test]
fn imported_schema_passing_alias_explicit_call_lowers() {
    let main = concat!(
        ">>> ./lib.td => @(queryAll => qa)\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= qa[Str](base, \"select 1\")\n",
        "stdout(out)\n",
    );
    let files = [("lib.td", XMOD_LIB), ("main.td", main)];
    let native = build_and_run_native_modules("f62b040_alias_native", &files, "main.td");
    assert!(!native.status.success());
    assert!(
        stderr_text(&native).contains("host capabilities are not available"),
        "native: expected the session-less rejection, got: {}",
        stderr_text(&native)
    );
}

/// An INFERABLE inference-form call to an imported schema-passing generic
/// is rejected with the explicit-call guidance. (A return-only type
/// parameter was already stopped by plain inference failure; a parameter
/// that also appears in an argument type inferred fine and sailed
/// through to the ABI mismatch.)
#[test]
fn imported_schema_passing_inferable_call_rejected() {
    let lib = r#"echoQuery[T] db: CageBuilder  x: T =
  db => InCage[_, "ping", @[]]() => Uncage[_, "all", T]() >=> rows
  rows
=> :T

<<< @(echoQuery)
"#;
    let main = concat!(
        ">>> ./lib.td => @(echoQuery)\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= echoQuery(base, \"hello\")\n",
        "stdout(out)\n",
    );
    let dir = unique_temp_dir("f62b040_inferable");
    write_file(&dir.join("lib.td"), lib);
    write_file(&dir.join("main.td"), main);
    let output = Command::new(taida_bin())
        .arg(dir.join("main.td"))
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success());
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("[E1510]")
            && stderr.contains("'echoQuery'")
            && stderr.contains("host-call Out slot"),
        "expected the explicit-arguments requirement, got: {stderr}"
    );
}

/// Cross-module forwarding: `outer[T] = queryAll[T](..)` where queryAll
/// is imported. The recursive import closure marks `outer` schema-passing
/// in its own module, the re-export carries it to the entry, and both
/// backends lower with the hidden schema threaded through two hops.
#[test]
fn cross_module_forwarding_lowers_and_enforces() {
    let mid = concat!(
        ">>> ./lib.td => @(queryAll)\n",
        "\n",
        "outer[T] db: CageBuilder  sql: Str =\n",
        "  queryAll[T](db, sql) => r\n",
        "  r\n",
        "=> :T\n",
        "\n",
        "<<< @(outer)\n",
    );
    let main = concat!(
        ">>> ./mid.td => @(outer)\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= outer[Str](base, \"select 1\")\n",
        "stdout(out)\n",
    );
    let files = [("lib.td", XMOD_LIB), ("mid.td", mid), ("main.td", main)];
    let native = build_and_run_native_modules("f62b040_xfwd_native", &files, "main.td");
    assert!(!native.status.success());
    assert!(
        stderr_text(&native).contains("host capabilities are not available"),
        "native: expected the session-less rejection, got: {}",
        stderr_text(&native)
    );
    if let Some(wasm) = build_and_run_wasm_modules("f62b040_xfwd_wasm", &files, "main.td") {
        assert!(!wasm.status.success());
        assert!(
            stderr_text(&wasm).contains("HostCapabilityError"),
            "wasm: expected the host-capability rejection, got: {}",
            stderr_text(&wasm)
        );
    } else {
        eprintln!("wasmtime not installed; skipping the wasm leg");
    }

    // The forwarding generic itself demands explicit type arguments.
    let main_inf = concat!(
        ">>> ./mid.td => @(outer)\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= outer(base, \"select 1\")\n",
        "stdout(out)\n",
    );
    let dir = unique_temp_dir("f62b040_xfwd_inference");
    write_file(&dir.join("lib.td"), XMOD_LIB);
    write_file(&dir.join("mid.td"), mid);
    write_file(&dir.join("main.td"), main_inf);
    let output = Command::new(taida_bin())
        .arg(dir.join("main.td"))
        .output()
        .expect("run taida interpreter");
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success());
    assert!(
        stderr_text(&output).contains("[E1510]") && stderr_text(&output).contains("'outer'"),
        "expected the explicit-arguments requirement on the imported forwarder, got: {}",
        stderr_text(&output)
    );
}

/// Two local forwarding hops (`middle[T] = outer[T](..) = inner[T](..)`):
/// the fixpoint closes over chains, not just single edges.
#[test]
fn two_hop_local_forwarding_lowers() {
    let source = concat!(
        "inner[T] db: CageBuilder  sql: Str =\n",
        "  db => InCage[_, \"prepare\", @[sql]]() => Uncage[_, \"all\", T]() >=> rows\n",
        "  rows\n",
        "=> :T\n",
        "\n",
        "outer[T] db: CageBuilder  sql: Str =\n",
        "  inner[T](db, sql) => r\n",
        "  r\n",
        "=> :T\n",
        "\n",
        "middle[T] db: CageBuilder  sql: Str =\n",
        "  outer[T](db, sql) => r\n",
        "  r\n",
        "=> :T\n",
        "\n",
        "cap <= HostCapability[\"CAP\", \"mock/kind\"]()\n",
        "base <= Cage[cap]()\n",
        "out <= middle[Str](base, \"select 1\")\n",
        "stdout(out)\n",
    );
    let run = build_and_run_native("f62b040_two_hop", source, false);
    assert!(!run.status.success());
    assert!(
        stderr_text(&run).contains("host capabilities are not available"),
        "expected the session-less rejection, got: {}",
        stderr_text(&run)
    );
}

// ── F62B-041: the combined bracket form on the build backends ────

/// `fn[1](2)` is rejected at build time on native/wasm/js too — the
/// interpreter-only guard left the lowering and JS paths silently
/// dropping the bracket on unchecked builds.
#[test]
fn non_generic_bracket_with_call_args_rejected_on_build_backends() {
    let source = "f x: Int = x => :Int\nstdout(f[7](3))\n";
    let dir = unique_temp_dir("f62b041_native");
    let src = dir.join("main.td");
    write_file(&src, source);

    let native = Command::new(taida_bin())
        .arg("build")
        .arg("--no-check")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("main_bin"))
        .output()
        .expect("native build");
    assert!(!native.status.success());
    assert!(
        stderr_text(&native).contains("cannot take both bracket values and call arguments"),
        "native: expected the combined-form rejection, got: {}",
        stderr_text(&native)
    );

    let wasm = Command::new(taida_bin())
        .arg("build")
        .arg("wasm-wasi")
        .arg("--no-check")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("main.wasm"))
        .output()
        .expect("wasm build");
    assert!(!wasm.status.success());
    assert!(
        stderr_text(&wasm).contains("cannot take both bracket values and call arguments"),
        "wasm: expected the combined-form rejection, got: {}",
        stderr_text(&wasm)
    );

    let js = Command::new(taida_bin())
        .arg("build")
        .arg("js")
        .arg("--no-check")
        .arg(&src)
        .arg("-o")
        .arg(dir.join("main.js"))
        .output()
        .expect("js build");
    let _ = fs::remove_dir_all(&dir);
    assert!(!js.status.success());
    assert!(
        stderr_text(&js).contains("cannot take both bracket values and call arguments"),
        "js: expected the combined-form rejection, got: {}",
        stderr_text(&js)
    );
}

/// The legacy positional bracket call still lowers everywhere.
#[test]
fn non_generic_legacy_bracket_call_still_lowers() {
    let run = build_and_run_native(
        "f62b041_legacy_native",
        "f x: Int = x => :Int\nstdout(f[3]())\n",
        true,
    );
    assert!(
        run.status.success(),
        "legacy bracket call must run natively\nstderr={}",
        stderr_text(&run)
    );
    assert_eq!(stdout_text(&run), "3\n");
}
