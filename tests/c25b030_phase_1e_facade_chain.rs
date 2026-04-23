//! C25B-030 Phase 1E-α: regression guard for addon facade-internal
//! `>>> ./X.td` relative imports.
//!
//! Background: Phase 1E-α extends `load_addon_facade_for_lower` in
//! `src/codegen/lower/imports.rs` so an addon facade can pull
//! aliases / pack bindings from sibling `.td` files through a
//! relative `>>>` import. The previous RC2.5 v1 loader rejected
//! any `Statement::Import` as an "unsupported top-level construct".
//!
//! Supported in Phase 1E-α:
//!
//! - `>>> ./child.td => @(sym1, sym2)` — relative facade import
//!   scoped to the listed symbols. Chainable across multiple files.
//! - Child facades may declare additional `>>>` imports pointing at
//!   their own siblings, recursively.
//!
//! Explicitly out of scope (tracked for Phase 1E-β / 1E-γ):
//!
//! - Function definitions inside a facade (`Name args = body`).
//! - TypeDef / EnumDef / MoldDef statements inside a facade.
//! - Non-relative `>>>` paths (`>>> taida-lang/foo`, `>>> npm:*`).
//! - `<<< <path>` re-export.
//! - Facade aliases to names that are not in the addon manifest's
//!   `[functions]` table.
//!
//! Each negative test pins the exact error-message substring so
//! downstream consumers (editor integrations, CI error matchers)
//! have stable contracts.

#![cfg(feature = "native")]

use std::path::{Path, PathBuf};
use std::process::Command;

// ── Shared fixture helpers ──────────────────────────────────

fn taida_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_taida"))
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "c25b030_1e_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos
    ))
}

const ADDON_TOML: &str = r#"abi = 1
entry = "taida_addon_get_v1"
package = "taida-lang/terminal"
library = "taida_lang_terminal"

[functions]
terminalSize = 0
readKey = 0
"#;

/// Lay down a minimal `.taida/deps/taida-lang/terminal/` directory
/// mirroring `rc2_5_native_terminal_phase2.rs::write_terminal_fixture_with_facade`
/// but with a caller-supplied `taida/terminal.td` facade text so each
/// test can exercise a different facade shape.
fn write_terminal_fixture(project: &Path, facade_td: &str, extra_files: &[(&str, &str)]) {
    std::fs::write(
        project.join("packages.tdm"),
        "name <= \"c25b030-1e-test\"\nversion <= \"0.1.0\"\n",
    )
    .expect("write project packages.tdm");

    let pkg = project
        .join(".taida")
        .join("deps")
        .join("taida-lang")
        .join("terminal");
    std::fs::create_dir_all(pkg.join("native")).expect("create native dir");
    std::fs::create_dir_all(pkg.join("taida")).expect("create taida dir");

    std::fs::write(pkg.join("native").join("addon.toml"), ADDON_TOML).expect("write addon.toml");
    std::fs::write(pkg.join("taida").join("terminal.td"), facade_td)
        .expect("write terminal facade");

    for (name, body) in extra_files {
        std::fs::write(pkg.join("taida").join(name), body).expect("write sibling facade file");
    }

    // Zero-byte placeholder cdylib — compile-time existence check only.
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    std::fs::write(
        pkg.join("native")
            .join(format!("libtaida_lang_terminal.{}", suffix)),
        b"",
    )
    .expect("write placeholder cdylib");

    std::fs::write(
        pkg.join("packages.tdm"),
        "name <= \"taida-lang/terminal\"\nversion <= \"0.1.0\"\n",
    )
    .expect("write pkg packages.tdm");
}

fn build_native(project: &Path, main_td: &str) -> (bool, String, String) {
    std::fs::write(project.join("main.td"), main_td).expect("write main.td");
    let output = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("native")
        .arg(project.join("main.td"))
        .arg("-o")
        .arg(project.join("main.bin"))
        .current_dir(project)
        .output()
        .expect("taida binary must run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// ── Positive: `>>> ./child.td => @(pack)` merges a child's pack binding ──

/// The canonical exercise for Phase 1E-α: the root facade delegates
/// a pack binding (`KeyKind`) to a sibling child file. The importer
/// imports the re-exported `KeyKind` symbol from `taida-lang/terminal`
/// and accesses its `Char` variant. The native build must accept
/// this even though `KeyKind` was never written in `terminal.td`.
#[test]
fn phase_1e_alpha_child_pack_binding_is_surfaced_via_parent_facade() {
    let project = unique_temp_dir("pack_child");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./keys.td => @(KeyKind)

TerminalSize <= terminalSize

<<< @(TerminalSize, KeyKind)
"#;
    let keys_td = r#"
KeyKind <= @(
  Char  <= 0
  Enter <= 1
)

<<< @(KeyKind)
"#;
    write_terminal_fixture(&project, terminal_td, &[("keys.td", keys_td)]);

    let main_td = r#">>> taida-lang/terminal => @(KeyKind)
stdout(`phase1e cha=${KeyKind.Char} ent=${KeyKind.Enter}`)
"#;

    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-α: re-exported child pack binding must compile on native. \
         stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        project.join("main.bin").exists(),
        "native build must produce an executable. stderr={}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Aliases declared in a child file are merged into the parent
/// facade's surface, so a user importing `TerminalSize` (defined as
/// `TerminalSize <= terminalSize` inside `aliases.td`) resolves to
/// the same addon sentinel as if it had been written in `terminal.td`.
#[test]
fn phase_1e_alpha_child_alias_binding_is_surfaced_via_parent_facade() {
    let project = unique_temp_dir("alias_child");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./aliases.td => @(TerminalSize)

KeyKind <= @(Char <= 0, Enter <= 1)

<<< @(TerminalSize, KeyKind)
"#;
    let aliases_td = r#"
TerminalSize <= terminalSize

<<< @(TerminalSize)
"#;
    write_terminal_fixture(&project, terminal_td, &[("aliases.td", aliases_td)]);

    let main_td = r#">>> taida-lang/terminal => @(TerminalSize)
size <= TerminalSize[]()
stdout(`phase1e cols=${size.cols}`)
"#;

    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-α: child-file alias to addon fn must flow through native. \
         stdout={}, stderr={}",
        stdout, stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Chained `>>>` across three facade files still surfaces the
/// leaf's pack binding at the root. Exercises the recursive
/// `load_addon_facade_file` entry.
#[test]
fn phase_1e_alpha_multi_level_chain_is_surfaced_via_parent_facade() {
    let project = unique_temp_dir("multi_level");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./mid.td => @(KeyKind)

<<< @(KeyKind)
"#;
    let mid_td = r#"
>>> ./leaf.td => @(KeyKind)

<<< @(KeyKind)
"#;
    let leaf_td = r#"
KeyKind <= @(
  Char  <= 0
  Enter <= 1
)

<<< @(KeyKind)
"#;
    write_terminal_fixture(
        &project,
        terminal_td,
        &[("mid.td", mid_td), ("leaf.td", leaf_td)],
    );

    let main_td = r#">>> taida-lang/terminal => @(KeyKind)
stdout(`cha=${KeyKind.Char}`)
"#;
    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-α: 3-level `>>>` chain must resolve at native build. \
         stdout={}, stderr={}",
        stdout, stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

// ── Negative: error contracts for unsupported constructs ──

/// `>>>` paths other than relative (`./`, `../`) are rejected with
/// a message naming the facade file and the offending path.
#[test]
fn phase_1e_alpha_non_relative_import_is_rejected_with_named_path() {
    let project = unique_temp_dir("non_rel");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> taida-lang/other => @(Foo)

<<< @(Foo)
"#;
    write_terminal_fixture(&project, terminal_td, &[]);

    let main_td = r#">>> taida-lang/terminal => @(Foo)
stdout(`${Foo.bar}`)
"#;

    let (ok, _stdout, stderr) = build_native(&project, main_td);
    assert!(!ok, "non-relative facade import must fail the build");
    assert!(
        stderr.contains("only relative `>>> ./X.td`"),
        "error must name the Phase 1E-α restriction, got: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Phase 1E-β: a child facade file's zero-arg FuncDef whose body is
/// a simple string literal (matches the real
/// `.dev/official-package-repos/terminal/taida/ansi.td::ClearScreen`)
/// must lower to a callable native symbol. User code imports the
/// facade-exported name via `>>> taida-lang/terminal => @(ClearScreen)`
/// and invokes it just like any other function.
///
/// This is the minimal end-to-end exercise for Phase 1E-β: a
/// facade FuncDef survives the facade loader, gets collected into
/// `addon_facade_funcs`, is lowered during the 2nd pass of the
/// main module's `lower_program` under a mangled link symbol, and
/// resolves through `imported_func_links` at the user call site.
#[test]
fn phase_1e_beta_child_zero_arg_funcdef_lowers_to_native() {
    let project = unique_temp_dir("beta_child_fn_zero_arg");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./ansi.td => @(ClearScreen)

<<< @(ClearScreen)
"#;
    // `ansi.td` uses a typed zero-arg function form
    // (`Name = <expr> => :Str`), which the parser represents as a
    // FuncDef — matches the real `.dev/official-package-repos/terminal/taida/ansi.td`.
    let ansi_td = r#"
ClearScreen =
  "clear-screen-marker"
=> :Str

<<< @(ClearScreen)
"#;
    write_terminal_fixture(&project, terminal_td, &[("ansi.td", ansi_td)]);

    let main_td = r#">>> taida-lang/terminal => @(ClearScreen)
stdout(ClearScreen())
"#;

    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-β: child facade with FuncDef must build on native. \
         stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        project.join("main.bin").exists(),
        "native build must produce an executable. stderr={}",
        stderr
    );

    let run = Command::new(project.join("main.bin"))
        .current_dir(&project)
        .output()
        .expect("run produced binary");
    assert!(
        run.status.success(),
        "native binary must run. stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run_stdout.contains("clear-screen-marker"),
        "FuncDef body must be executed by the native binary. got: {}",
        run_stdout
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Phase 1E-β: a FuncDef with non-trivial parameters and a
/// pipeline-style body that uses `Error().throw()` guards plus
/// string concatenation + `toString()` method calls (mirrors the
/// real `ansi.td::CursorMoveTo`). Confirms that facade FuncDef
/// lowering covers the typed-parameter + guard-branch combinator
/// shape used throughout the terminal addon.
#[test]
fn phase_1e_beta_child_funcdef_with_args_and_guards() {
    let project = unique_temp_dir("beta_child_fn_args");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./ansi.td => @(CursorMoveTo)

<<< @(CursorMoveTo)
"#;
    let ansi_td = r#"
CursorMoveTo col row =
  | col < 1 |> Error(type <= "CursorMoveInvalidPosition", message <= "col must be >= 1").throw()
  | row < 1 |> Error(type <= "CursorMoveInvalidPosition", message <= "row must be >= 1").throw()
  | _ |> "\x1b[" + row.toString() + ";" + col.toString() + "H"
=> :Str

<<< @(CursorMoveTo)
"#;
    write_terminal_fixture(&project, terminal_td, &[("ansi.td", ansi_td)]);

    let main_td = r#">>> taida-lang/terminal => @(CursorMoveTo)
stdout(CursorMoveTo(10, 5))
"#;
    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-β: parameterised facade FuncDef with guards must build. \
         stdout={}, stderr={}",
        stdout, stderr
    );

    let run = Command::new(project.join("main.bin"))
        .current_dir(&project)
        .output()
        .expect("run produced binary");
    assert!(run.status.success(), "binary must exit 0");
    let run_stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run_stdout.contains("\x1b[5;10H"),
        "CursorMoveTo(10, 5) must emit ESC[5;10H. got: {:?}",
        run_stdout
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Phase 1E-β: a top-level facade FuncDef declared directly in
/// `terminal.td` (not in a child file) must also lower cleanly.
/// Exercises the code path where `load_addon_facade_for_lower`
/// harvests FuncDefs without the `>>>` indirection — the simplest
/// possible Phase-1E-β surface.
#[test]
fn phase_1e_beta_toplevel_funcdef_lowers_to_native() {
    let project = unique_temp_dir("beta_toplevel_fn");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
Greet who =
  "hello " + who + "!"
=> :Str

<<< @(Greet)
"#;
    write_terminal_fixture(&project, terminal_td, &[]);

    let main_td = r#">>> taida-lang/terminal => @(Greet)
stdout(Greet("world"))
"#;
    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-β: top-level facade FuncDef must build. stdout={}, stderr={}",
        stdout, stderr
    );

    let run = Command::new(project.join("main.bin"))
        .current_dir(&project)
        .output()
        .expect("run produced binary");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(out.contains("hello world!"), "got: {:?}", out);

    let _ = std::fs::remove_dir_all(&project);
}

/// Phase 1E-β: import aliasing (`>>> ... => @(Orig: Local)`) must
/// still work when the imported symbol is a FuncDef from the
/// facade. The mangled link symbol stays bound to `Orig`; only
/// the user-facing name is rewritten via `imported_func_links`.
#[test]
fn phase_1e_beta_funcdef_user_import_alias_is_honoured() {
    let project = unique_temp_dir("beta_import_alias");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
Greet who =
  "hi " + who
=> :Str

<<< @(Greet)
"#;
    write_terminal_fixture(&project, terminal_td, &[]);

    let main_td = r#">>> taida-lang/terminal => @(Greet: MyGreet)
stdout(MyGreet("Taida"))
"#;
    let (ok, stdout, stderr) = build_native(&project, main_td);
    assert!(
        ok,
        "Phase 1E-β: import alias for facade FuncDef must build. \
         stdout={}, stderr={}",
        stdout, stderr
    );

    let run = Command::new(project.join("main.bin"))
        .current_dir(&project)
        .output()
        .expect("run produced binary");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(out.contains("hi Taida"), "got: {:?}", out);

    let _ = std::fs::remove_dir_all(&project);
}

/// Phase 1E-β: TypeDef / EnumDef / MoldDef statements inside a
/// facade remain rejected with a stable Phase-1E-γ-referencing
/// message. Mirrors the original Phase 1E-α test shape but with
/// the follow-up phase pointer rotated forward.
#[test]
fn phase_1e_beta_typedef_in_facade_is_still_rejected() {
    let project = unique_temp_dir("beta_typedef_reject");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
MyRecord = @(x: Int, y: Int)

<<< @(MyRecord)
"#;
    write_terminal_fixture(&project, terminal_td, &[]);

    let main_td = r#">>> taida-lang/terminal => @(MyRecord)
stdout(`${MyRecord}`)
"#;
    let (ok, _stdout, stderr) = build_native(&project, main_td);
    assert!(!ok, "TypeDef inside a facade must still fail the build");
    assert!(
        stderr.contains("C25B-030 Phase 1E-γ"),
        "error must point to Phase 1E-γ as the follow-up, got: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// A symbol requested via `>>> ./child.td => @(Missing)` that the
/// child does not actually produce must fail with a precise error
/// naming the symbol, the child file, and the facade's canonical
/// import path.
#[test]
fn phase_1e_alpha_missing_child_symbol_is_rejected_with_precise_message() {
    let project = unique_temp_dir("missing_child_sym");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./keys.td => @(Missing)

<<< @(Missing)
"#;
    let keys_td = r#"
KeyKind <= @(Char <= 0)

<<< @(KeyKind)
"#;
    write_terminal_fixture(&project, terminal_td, &[("keys.td", keys_td)]);

    let main_td = r#">>> taida-lang/terminal => @(Missing)
stdout(`${Missing.x}`)
"#;

    let (ok, _stdout, stderr) = build_native(&project, main_td);
    assert!(!ok, "missing-from-child symbol must fail the build");
    assert!(
        stderr.contains("requested symbol 'Missing'"),
        "error must name the missing symbol, got: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// Circular facade imports across a 2-file chain must surface as a
/// deterministic compile error instead of hanging the build.
#[test]
fn phase_1e_alpha_circular_child_import_is_rejected() {
    let project = unique_temp_dir("circular");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
>>> ./keys.td => @(KeyKind)

<<< @(KeyKind)
"#;
    let keys_td = r#"
>>> ./terminal.td => @(KeyKind)

KeyKind <= @(Char <= 0)

<<< @(KeyKind)
"#;
    write_terminal_fixture(&project, terminal_td, &[("keys.td", keys_td)]);

    let main_td = r#">>> taida-lang/terminal => @(KeyKind)
stdout(`cha=${KeyKind.Char}`)
"#;

    let (ok, _stdout, stderr) = build_native(&project, main_td);
    assert!(!ok, "circular facade import must be detected");
    assert!(
        stderr.contains("circular facade import"),
        "error must name circular detection, got: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}

/// `<<< <path>` re-export still fails, but with a Phase-1E-α-aware
/// message that matches the new error text.
#[test]
fn phase_1e_alpha_export_with_path_is_still_rejected() {
    let project = unique_temp_dir("export_path");
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).unwrap();

    let terminal_td = r#"
KeyKind <= @(Char <= 0)

<<< ./terminal.td
"#;
    write_terminal_fixture(&project, terminal_td, &[]);

    let main_td = r#">>> taida-lang/terminal => @(KeyKind)
stdout(`cha=${KeyKind.Char}`)
"#;

    let (ok, _stdout, stderr) = build_native(&project, main_td);
    assert!(!ok, "`<<< <path>` must still be rejected");
    assert!(
        stderr.contains("re-export which is not") || stderr.contains("re-export which is  not"),
        "error should explain the re-export limitation, got: {}",
        stderr
    );

    let _ = std::fs::remove_dir_all(&project);
}
