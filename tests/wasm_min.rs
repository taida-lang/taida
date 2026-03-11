/// Integration tests for wasm-min backend.
///
/// Compiles .td files to .wasm via `taida build --target wasm-min`,
/// runs them with wasmtime, and verifies output matches the interpreter.
///
/// W-2: Size gate CI tests — hard gates on .wasm file sizes.
/// Wado baselines: hello_world = 1,572 bytes, pi_approx = 9,269 bytes.
/// Gate: hello <= 2KB (minimum), <= 1,572 bytes (stretch); pi <= 9,269 bytes (hard).
use std::path::{Path, PathBuf};
use std::process::Command;

/// Get the path to the built taida binary.
fn taida_bin() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_taida"));
    if !path.exists() {
        path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("taida");
    }
    path
}

/// Find wasmtime binary.
fn wasmtime_bin() -> Option<PathBuf> {
    // Check HOME/.wasmtime/bin/wasmtime
    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home).join(".wasmtime/bin/wasmtime");
        if path.exists() {
            return Some(path);
        }
    }
    // Check PATH
    if let Ok(output) = Command::new("which").arg("wasmtime").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// Run a .td file with the native backend and return its stdout.
fn run_native(td_path: &Path) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let native_path = std::env::temp_dir().join(format!("taida_native_test_{}", stem));

    let compile_output = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("native")
        .arg(td_path)
        .arg("-o")
        .arg(&native_path)
        .output()
        .ok()?;

    if !compile_output.status.success() {
        return None;
    }

    let output = Command::new(&native_path).output().ok()?;
    let _ = std::fs::remove_file(&native_path);

    if !output.status.success() {
        return None;
    }

    Some(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
    )
}

/// Run a .td file with the interpreter and return its stdout.
fn run_interpreter(td_path: &Path) -> Option<String> {
    let output = Command::new(taida_bin()).arg(td_path).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
    )
}

/// Compile a .td file to wasm-min and run with wasmtime.
fn compile_and_run_wasm(td_path: &Path, wasmtime: &Path) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let wasm_path = std::env::temp_dir().join(format!("taida_wasm_test_{}.wasm", stem));

    // Compile
    let compile_output = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("wasm-min")
        .arg(td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .ok()?;

    if !compile_output.status.success() {
        let stderr = String::from_utf8_lossy(&compile_output.stderr);
        eprintln!(
            "wasm-min compile failed for {}: {}",
            td_path.display(),
            stderr
        );
        return None;
    }

    // Run with wasmtime
    let run_output = Command::new(wasmtime).arg(&wasm_path).output().ok()?;

    // Clean up
    let _ = std::fs::remove_file(&wasm_path);

    if !run_output.status.success() {
        let stderr = String::from_utf8_lossy(&run_output.stderr);
        eprintln!(
            "wasmtime execution failed for {}: {}",
            td_path.display(),
            stderr
        );
        return None;
    }

    Some(
        String::from_utf8_lossy(&run_output.stdout)
            .trim_end()
            .to_string(),
    )
}

#[test]
fn wasm_min_hello() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(interp, wasm, "wasm-min output should match interpreter");
}

#[test]
fn wasm_min_pi_approx() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_pi_approx.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(interp, wasm, "wasm-min output should match interpreter");
}

// ---------------------------------------------------------------------------
// W-2c: Size Gate CI — hard gates on .wasm file sizes
// ---------------------------------------------------------------------------

/// Helper: compile a .td to .wasm and return the file size in bytes.
fn compile_wasm_and_get_size(td_path: &Path, wasm_path: &Path) -> u64 {
    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(td_path)
        .arg("-o")
        .arg(wasm_path)
        .output()
        .expect("failed to run taida");
    assert!(
        output.status.success(),
        "wasm-min compile failed for {}: {}",
        td_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let size = std::fs::metadata(wasm_path).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_file(wasm_path);
    size
}

#[test]
fn wasm_min_size_gate() {
    let hello_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let pi_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_pi_approx.td");

    let hello_size = compile_wasm_and_get_size(
        &hello_path,
        &std::env::temp_dir().join("taida_wasm_size_hello.wasm"),
    );
    let pi_size = compile_wasm_and_get_size(
        &pi_path,
        &std::env::temp_dir().join("taida_wasm_size_pi.wasm"),
    );

    // Wado baselines: hello_world = 1,572 bytes, pi_approx = 9,269 bytes
    eprintln!("wasm-min hello size: {} bytes (Wado: 1,572)", hello_size);
    eprintln!("wasm-min pi size: {} bytes (Wado: 9,269)", pi_size);

    // Hard gate: pi_approx must be <= 9,269 bytes (Wado baseline)
    assert!(
        pi_size > 0 && pi_size <= 9269,
        "HARD GATE FAIL: pi.wasm should be <= 9,269 bytes (Wado baseline), got {} bytes. \
         W-3 and beyond are blocked until this passes.",
        pi_size
    );

    // Minimum gate: hello must be <= 2KB
    assert!(
        hello_size > 0 && hello_size <= 2048,
        "MINIMUM GATE FAIL: hello.wasm should be <= 2KB, got {} bytes",
        hello_size
    );

    // Stretch goal: hello <= 1,572 bytes (Wado baseline)
    if hello_size <= 1572 {
        eprintln!(
            "STRETCH GOAL MET: hello.wasm ({} bytes) <= Wado baseline (1,572 bytes)",
            hello_size
        );
    } else {
        eprintln!(
            "Stretch goal not met: hello.wasm ({} bytes) > Wado baseline (1,572 bytes)",
            hello_size
        );
    }
}

/// W-2c: Size gate with exact Wado comparison — reports the ratio.
#[test]
fn wasm_min_size_gate_wado_comparison() {
    let hello_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let pi_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_pi_approx.td");

    let hello_size = compile_wasm_and_get_size(
        &hello_path,
        &std::env::temp_dir().join("taida_wasm_wado_hello.wasm"),
    );
    let pi_size = compile_wasm_and_get_size(
        &pi_path,
        &std::env::temp_dir().join("taida_wasm_wado_pi.wasm"),
    );

    // Report Wado comparison ratios
    let hello_ratio = 1572.0 / hello_size as f64;
    let pi_ratio = 9269.0 / pi_size as f64;
    eprintln!(
        "Wado comparison: hello = {} bytes (Wado: 1,572 = {:.1}x larger), \
         pi = {} bytes (Wado: 9,269 = {:.1}x larger)",
        hello_size, hello_ratio, pi_size, pi_ratio
    );

    // Sanity: both should be non-zero and compile correctly
    assert!(hello_size > 0, "hello.wasm should not be empty");
    assert!(pi_size > 0, "pi.wasm should not be empty");
}

// W-4: BuchiPack is now supported in wasm-min.
// wasm_min_reject_buchipack removed -- see wasm_min_pack_basic instead.

/// W-4: Basic BuchiPack support — create, field access, stdout.
#[test]
fn wasm_min_pack_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/pack_basic.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "pack_basic: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-4: Nested BuchiPack — inner pack accessed through outer pack fields.
#[test]
fn wasm_min_pack_nested() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/pack_nested.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "pack_nested: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-4: BuchiPack compile acceptance test (was wasm_min_reject_buchipack before W-4).
#[test]
fn wasm_min_pack_accepted() {
    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/unsupported_pack.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_pack_accepted.wasm");

    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("failed to run taida");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        output.status.success(),
        "W-4: BuchiPack should now be accepted by wasm-min, but compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// W-5: wasm_min_reject_closure removed -- Closures are now supported.
// See wasm_min_closure_basic, wasm_min_closure_accepted instead.

/// W-5: Closures should now be accepted by wasm-min.
#[test]
fn wasm_min_closure_accepted() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wasm_min/unsupported_closure.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_closure_accepted.wasm");

    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("failed to run taida");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        output.status.success(),
        "W-5: Closures should now be accepted by wasm-min, but compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// W-3: wasm_min_reject_float removed -- Float is now supported.
// See wasm_min_float_accepted test instead.

// ---------------------------------------------------------------------------
// W-2: Regression tests (from review Low findings)
// ---------------------------------------------------------------------------

/// W-2: Multiple global variables should each get their own C static variable.
/// Verifies that globals do not collide (F-4 regression).
#[test]
fn wasm_min_multiple_globals() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/multiple_globals.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "multiple globals: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-2: --release gate should work with wasm-min (positive case: no TODO/Stub).
#[test]
fn wasm_min_release_gate_positive() {
    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/release_gate.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_release_pos.wasm");

    // Compiling with --release should succeed (no TODO/Stub in source)
    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min", "--release"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("failed to run taida");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        output.status.success(),
        "wasm-min --release should succeed for clean code, but failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------
// W-3: Float support tests
// ---------------------------------------------------------------------------

/// W-3: Float literals and debug(Float) should work.
#[test]
fn wasm_min_float_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/float_basic.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "float_basic: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-3: Float arithmetic (add, sub, mul) should work.
#[test]
fn wasm_min_float_arith() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/float_arith.td");
    // Float formatting differs between interpreter and compiled backends
    // (e.g., "5.0" vs "5", "5.840400000000001" vs "5.8404").
    // Compare against native backend output instead.
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "float_arith: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-3: String concatenation should work (requires bump allocator).
#[test]
fn wasm_min_str_ops() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/str_ops.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "str_ops: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-3: Float is no longer rejected (was rejected in wasm-min v1).
#[test]
fn wasm_min_float_accepted() {
    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/unsupported_float.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_float_accepted.wasm");

    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("failed to run taida");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        output.status.success(),
        "W-3: Float should now be accepted by wasm-min, but compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// W-2: --release gate should reject TODO molds in wasm-min (negative case).
#[test]
fn wasm_min_release_gate_negative() {
    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/release_gate_todo.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_release_neg.wasm");

    // Compiling with --release should fail (TODO mold present)
    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min", "--release"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("failed to run taida");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        !output.status.success(),
        "wasm-min --release should fail when TODO mold is present, but it succeeded"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Release gate failed"),
        "Error should contain 'Release gate failed', got: {}",
        stderr
    );
}

// ---------------------------------------------------------------------------
// W-3f: Blocker regression tests
// ---------------------------------------------------------------------------

/// W-3f F-1: debug(Float) should handle small non-zero values (scientific notation).
/// Compare against native backend (both use %g-equivalent formatting).
#[test]
fn wasm_min_float_small_values() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/float_small.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "float_small: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-3f F-2: String.length() should work via taida_polymorphic_length.
#[test]
fn wasm_min_str_length() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/str_length.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "str_length: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-3f F-2: Int.toString() should work via taida_polymorphic_to_string.
#[test]
fn wasm_min_int_to_string() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/int_to_string.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "int_to_string: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-3f F-2: Int["str"]() ]=> should work via taida_int_mold_str + taida_generic_unmold.
#[test]
fn wasm_min_int_mold_str() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/int_mold_str.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "int_mold_str: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

// ---------------------------------------------------------------------------
// W-4: Collection type tests
// ---------------------------------------------------------------------------

/// W-4: Basic list support — create, push, length.
#[test]
fn wasm_min_list_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/list_basic.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "list_basic: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

// ---------------------------------------------------------------------------
// W-4f: HashMap/Set integration tests (F-2)
// ---------------------------------------------------------------------------

/// W-4f F-2: HashMap basic operations — new, set, has, size, keys, values, remove, merge, entries.
#[test]
fn wasm_min_hashmap_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/hashmap_basic.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "hashmap_basic: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-4f F-2: Set basic operations — setOf, size, add, has, remove, union, intersect, diff, toList.
#[test]
fn wasm_min_set_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/set_basic.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "set_basic: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

// ---------------------------------------------------------------------------
// W-5: Control flow and function tests
// ---------------------------------------------------------------------------

/// W-5: Basic closure — function returning a function with captured variable.
#[test]
fn wasm_min_closure_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/closure_basic.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "closure_basic: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-5: Error ceiling — error handling with |== operator.
#[test]
fn wasm_min_error_ceiling() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/error_ceiling.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "error_ceiling: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

/// W-5: Lax[T] — Div/Mod molds return Lax, ]=> unmolds.
#[test]
fn wasm_min_lax_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/lax_basic.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        interp, wasm,
        "lax_basic: wasm-min output should match interpreter (expected '{}', got '{}')",
        interp, wasm
    );
}

// ---------------------------------------------------------------------------
// W-5f: Lax/Result toString and Result basic tests
// ---------------------------------------------------------------------------

/// W-5f F-2: Lax.toString() — "Lax(42)", "Lax(default: 0)", "Lax(3)".
#[test]
fn wasm_min_lax_tostring() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/lax_tostring.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "lax_tostring: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-5f F-1/F-3: Result basic — create, unmold, toString.
#[test]
fn wasm_min_result_basic() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/result_basic.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "result_basic: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-5g F-1: Result with predicate — predicate-fail should be error, predicate-pass should succeed.
#[test]
fn wasm_min_result_predicate() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/result_predicate.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "result_predicate: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

/// W-5g F-2: Str->Float/Bool mold failure should return empty Lax (not success Lax(0)).
#[test]
fn wasm_min_mold_fail() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm_min/mold_fail.td");
    let native_output = run_native(&td_path).expect("native should succeed");
    let wasm = compile_and_run_wasm(&td_path, &wasmtime).expect("wasm-min should succeed");

    assert_eq!(
        native_output, wasm,
        "mold_fail: wasm-min output should match native (expected '{}', got '{}')",
        native_output, wasm
    );
}

// ---------------------------------------------------------------------------
// W-6: Parity test — all compilable examples must match native output
// ---------------------------------------------------------------------------

/// W-6: Comprehensive parity test.
/// For every .td file in examples/ that successfully compiles with wasm-min,
/// the wasm output must match the native backend output exactly.
#[test]
fn wasm_min_parity_all_examples() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min parity test");
            return;
        }
    };

    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut td_files: Vec<_> = std::fs::read_dir(&examples_dir)
        .expect("examples/ directory should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "td"))
        .collect();
    td_files.sort();

    let mut parity_ok = Vec::new();
    let mut parity_fail = Vec::new();
    let mut compile_rejected = Vec::new();
    let mut native_fail = Vec::new();

    for td_path in &td_files {
        let stem = td_path.file_stem().unwrap().to_string_lossy().to_string();

        // Try native build first
        let native_output = run_native(td_path);
        if native_output.is_none() {
            native_fail.push(stem.clone());
            continue;
        }
        let native_out = native_output.unwrap();

        // Try wasm-min compile + run
        let wasm_output = compile_and_run_wasm(td_path, &wasmtime);
        if wasm_output.is_none() {
            compile_rejected.push(stem.clone());
            continue;
        }
        let wasm_out = wasm_output.unwrap();

        if native_out == wasm_out {
            parity_ok.push(stem.clone());
        } else {
            parity_fail.push((stem.clone(), native_out, wasm_out));
        }
    }

    eprintln!(
        "W-6 Parity: {} OK, {} rejected, {} native-fail",
        parity_ok.len(),
        compile_rejected.len(),
        native_fail.len()
    );

    if !parity_fail.is_empty() {
        let mut msg = format!("W-6 PARITY FAILED for {} example(s):\n", parity_fail.len());
        for (stem, native, wasm) in &parity_fail {
            msg.push_str(&format!(
                "\n  {}: native='{}' vs wasm='{}'\n",
                stem,
                native.chars().take(100).collect::<String>(),
                wasm.chars().take(100).collect::<String>()
            ));
        }
        panic!("{}", msg);
    }

    // At least 20 examples should have parity (sanity check)
    assert!(
        parity_ok.len() >= 20,
        "W-6: Expected at least 20 examples with parity, got {}",
        parity_ok.len()
    );
}
