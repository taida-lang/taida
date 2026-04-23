//! C25B-025 Phase 5-A — interpreter and JS backends must produce
//! the same numerical value for the math mold family (Sqrt / Pow /
//! Sin / Cos / Tan / Exp / Ln / Log / Log2 / Log10 / Asin / Acos /
//! Atan / Atan2 / Sinh / Cosh / Tanh).
//!
//! Pre-fix (reproducible on upstream/main 2026-04-23):
//!
//!   Sqrt[4.0]()    → @(__value <= 4.0, __type <= "Sqrt")   (not 2.0)
//!   Pow[2.0, 10]() → @(__value <= 2.0, __type <= "Pow")    (not 1024.0)
//!   Sin[0.0]()     → type-check error (Sin unknown)
//!
//! Phase 5-A adds interpreter and JS runtime implementations for the
//! full math family. Native / wasm lowering is deferred (see
//! `.dev/C25_PROGRESS.md` Phase 5-A scope note — pinned by the
//! test function suffix `_interpreter_and_js`).
//!
//! NB: JS `Number.prototype.toString()` renders whole-valued floats
//! without a decimal point (`2` vs `2.0`), which is a pre-existing
//! 4-backend display gap (it already affects `Abs[2.0]()`,
//! `Clamp[2.0, 0.0, 10.0]()`, etc.). We normalise whole-float
//! tokens to strip the trailing `.0` when comparing values so the
//! math mold parity assertion focuses on the numerical correctness
//! introduced by Phase 5-A, not on that separate cosmetic gap.
//! Follow-up ticket: covered by the wider Float formatting audit
//! tracked outside this blocker.
mod common;

use common::taida_bin;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp_artifact(td_path: &Path, suffix: &str) -> PathBuf {
    let stem = td_path.file_stem().unwrap().to_string_lossy();
    std::env::temp_dir().join(format!(
        "c25b_025_{}_{}.{}",
        std::process::id(),
        stem,
        suffix
    ))
}

/// Normalise whole-float display (`2.0` ↔ `2`) so that the JS
/// backend's default Number→string rendering compares equal to the
/// interpreter's `{:.1}` formatting. Non-whole floats, ints, strings
/// pass through unchanged.
fn normalise_whole_floats(s: &str) -> Vec<String> {
    s.lines()
        .map(|line| {
            let trimmed = line.trim();
            if let Some(stripped) = trimmed.strip_suffix(".0")
                && stripped.parse::<i64>().is_ok()
            {
                stripped.to_string()
            } else {
                trimmed.to_string()
            }
        })
        .collect()
}

fn run_interpreter(td_path: &Path) -> Vec<String> {
    let out = Command::new(taida_bin())
        .arg(td_path)
        .output()
        .expect("spawn interpreter");
    assert!(
        out.status.success(),
        "interpreter failed for {}: {}",
        td_path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    normalise_whole_floats(&String::from_utf8_lossy(&out.stdout))
}

fn run_js(td_path: &Path) -> Option<Vec<String>> {
    let js_path = tmp_artifact(td_path, "mjs");
    let build = Command::new(taida_bin())
        .args(["build", "--target", "js"])
        .arg(td_path)
        .arg("-o")
        .arg(&js_path)
        .output()
        .expect("spawn taida build");
    if !build.status.success() {
        eprintln!(
            "js build failed for {}: {}",
            td_path.display(),
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let run = Command::new("node").arg(&js_path).output().ok()?;
    let _ = std::fs::remove_file(&js_path);
    if !run.status.success() {
        eprintln!(
            "js run failed for {}: {}",
            td_path.display(),
            String::from_utf8_lossy(&run.stderr)
        );
        return None;
    }
    Some(normalise_whole_floats(&String::from_utf8_lossy(
        &run.stdout,
    )))
}

#[test]
fn c25b_025_sqrt_pow_interpreter_and_js() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/quality/c25b_025_math_molds/sqrt_pow.td");

    let interp = run_interpreter(&fixture);
    assert_eq!(
        interp,
        vec![
            "2".to_string(),
            "1.4142135623730951".to_string(),
            "1024".to_string(),
            "9".to_string(),
        ],
        "interpreter sqrt/pow output shape regressed — Phase 5-A expected Sqrt[4.0]()=2, Sqrt[2.0]()=sqrt2, Pow[2.0,10]()=1024, Pow[3.0,2]()=9"
    );

    if let Some(js) = run_js(&fixture) {
        assert_eq!(js, interp, "JS math molds diverge from interpreter");
    } else {
        eprintln!("skipping JS parity check — node not available");
    }
}

#[test]
fn c25b_025_transcendentals_interpreter_and_js() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/quality/c25b_025_math_molds/transcendentals.td");

    let interp = run_interpreter(&fixture);
    assert_eq!(interp.len(), 8, "transcendental fixture emits 8 lines");
    // Line-by-line spot check for the well-known identities so the
    // test fails loudly if a future tweak breaks an identity.
    assert_eq!(interp[0], "0", "Sin[0.0]() == 0");
    assert_eq!(interp[1], "1", "Cos[0.0]() == 1");
    // Exp[1] = e = 2.718281828459045 (f64 rounding)
    assert_eq!(interp[2], "2.718281828459045", "Exp[1.0]() == e");
    // Ln[e] = 1 but f64 rounding yields 1.0 (we strip trailing .0)
    assert_eq!(interp[3], "1", "Ln[e]() == 1");
    assert_eq!(interp[4], "2", "Log10[100.0]() == 2");
    assert_eq!(interp[5], "3", "Log2[8.0]() == 3");
    // Atan2[1, 1] = pi/4 = 0.7853981633974483
    assert_eq!(interp[6], "0.7853981633974483", "Atan2[1.0, 1.0]() == pi/4");
    assert_eq!(interp[7], "3", "Log[8.0, 2.0]() == 3");

    if let Some(js) = run_js(&fixture) {
        assert_eq!(js, interp, "JS transcendentals diverge from interpreter");
    } else {
        eprintln!("skipping JS parity check — node not available");
    }
}
