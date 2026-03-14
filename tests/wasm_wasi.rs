/// Integration tests for wasm-wasi backend.
///
/// Compiles .td files to .wasm via `taida build --target wasm-wasi`,
/// runs them with wasmtime, and verifies output matches Native/interpreter.
///
/// WW-2: Tests for env, file I/O, and stderr non-regression.
/// WW-3: Validation — parity, superset property, size checks.
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn wasmtime_bin() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let path = PathBuf::from(home).join(".wasmtime/bin/wasmtime");
        if path.exists() {
            return Some(path);
        }
    }
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

/// Compile a .td file to wasm-wasi and run with wasmtime.
/// `extra_args` are passed to wasmtime (e.g. --env, --dir).
fn compile_and_run_wasm_wasi(
    td_path: &Path,
    wasmtime: &Path,
    extra_args: &[&str],
) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let wasm_path = std::env::temp_dir().join(format!("taida_wasm_wasi_test_{}.wasm", stem));

    let compile_output = Command::new(taida_bin())
        .arg("build")
        .arg("--target")
        .arg("wasm-wasi")
        .arg(td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .ok()?;

    if !compile_output.status.success() {
        let stderr = String::from_utf8_lossy(&compile_output.stderr);
        eprintln!(
            "wasm-wasi compile failed for {}: {}",
            td_path.display(),
            stderr
        );
        return None;
    }

    let mut cmd = Command::new(wasmtime);
    cmd.arg("run");
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.arg("--").arg(&wasm_path);
    let run_output = cmd.output().ok()?;

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

/// Test: wasm-wasi compiles and runs the stderr fixture.
/// Verifies stdout output matches interpreter (stderr goes to fd=2).
#[test]
fn wasm_wasi_stderr() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_stderr.td");
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    let wasm =
        compile_and_run_wasm_wasi(&td_path, &wasmtime, &[]).expect("wasm-wasi should succeed");

    assert_eq!(
        interp, wasm,
        "wasm-wasi stderr output should match interpreter"
    );
}

/// Test: wasm-wasi EnvVar + allEnv.
/// Injects env vars via wasmtime --env and verifies output.
#[test]
fn wasm_wasi_env() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_env.td");

    // Run interpreter with the same env vars
    let interp_output = Command::new(taida_bin())
        .arg(&td_path)
        .env("TAIDA_TEST_A", "hello")
        .env("TAIDA_TEST_B", "world")
        .output()
        .expect("interpreter should run");
    assert!(interp_output.status.success(), "interpreter failed");
    let _interp = String::from_utf8_lossy(&interp_output.stdout)
        .trim_end()
        .to_string();

    // Compile wasm-wasi
    let wasm_path = std::env::temp_dir().join("taida_wasm_wasi_test_env.wasm");
    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-wasi"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");
    assert!(
        compile.status.success(),
        "wasm-wasi compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Run with wasmtime, injecting only our 2 env vars
    let run = Command::new(&wasmtime)
        .args([
            "run",
            "--env",
            "TAIDA_TEST_A=hello",
            "--env",
            "TAIDA_TEST_B=world",
            "--",
        ])
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let wasm = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    // Line 1: EnvVar unmold value
    let wasm_lines: Vec<&str> = wasm.lines().collect();
    assert!(
        wasm_lines.len() >= 2,
        "wasm output too short: {:?}",
        wasm_lines
    );
    assert_eq!(
        wasm_lines[0], "hello",
        "EnvVar should resolve TAIDA_TEST_A=hello"
    );

    // Line 2: allEnv().size() — wasmtime --env injects exactly 2 vars
    // Interpreter may see more host env vars, so we only check wasm side
    assert_eq!(
        wasm_lines[1], "2",
        "allEnv should see exactly 2 injected env vars"
    );
}

/// Test: wasm-wasi file I/O (Read, writeFile, Exists).
#[test]
fn wasm_wasi_file_io() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_file_io.td");

    // Run interpreter
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    // Clean up temp file from interpreter run
    let _ = std::fs::remove_file("_wasi_test_tmp.txt");

    // Compile wasm-wasi
    let wasm_path = std::env::temp_dir().join("taida_wasm_wasi_test_file_io.wasm");
    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-wasi"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");
    assert!(
        compile.status.success(),
        "wasm-wasi compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Run with wasmtime, granting access to current directory
    let run = Command::new(&wasmtime)
        .args(["run", "--dir=.", "--"])
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);
    // Clean up temp file from wasm run
    let _ = std::fs::remove_file("_wasi_test_tmp.txt");

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let wasm = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    // Verify writeFile + Read round-trip works
    assert_eq!(
        wasm.trim(),
        "hello from wasi",
        "wasm-wasi file I/O should write and read back content"
    );
    assert_eq!(
        interp, wasm,
        "wasm-wasi file I/O output should match interpreter"
    );
}

/// Test: wasm-wasi Exists[path]() — verifies both existing and non-existing paths.
#[test]
fn wasm_wasi_exists() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_exists.td");

    // Run interpreter
    let interp = run_interpreter(&td_path).expect("interpreter should succeed");
    // Clean up temp file from interpreter run
    let _ = std::fs::remove_file("_wasi_exists_test.txt");

    // Compile wasm-wasi
    let wasm_path = std::env::temp_dir().join("taida_wasm_wasi_test_exists.wasm");
    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-wasi"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");
    assert!(
        compile.status.success(),
        "wasm-wasi compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    // Run with wasmtime, granting access to current directory
    let run = Command::new(&wasmtime)
        .args(["run", "--dir=.", "--"])
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);
    // Clean up temp file from wasm run
    let _ = std::fs::remove_file("_wasi_exists_test.txt");

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let wasm = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    assert_eq!(
        wasm, "true\nfalse",
        "Exists should return true for existing file and false for non-existing"
    );
    assert_eq!(
        interp, wasm,
        "wasm-wasi Exists output should match interpreter"
    );
}

/// Test: wasm-wasi writeFile failure path — non-existent directory.
#[test]
fn wasm_wasi_write_failure() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_write_failure.td");

    // Compile and run wasm-wasi
    let wasm = compile_and_run_wasm_wasi(&td_path, &wasmtime, &["--dir=."])
        .expect("wasm-wasi should succeed");

    assert_eq!(
        wasm, "true",
        "writeFile to non-existent dir should report isError() = true"
    );
}

/// Test: wasm-wasi writeFile failure shape — validates error field names survive Result toString.
/// WFX-S1: ensures error shape (type, message, kind) is not silently broken.
#[test]
fn wasm_wasi_write_failure_shape() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi tests");
            return;
        }
    };

    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_write_failure_shape.td");

    // Compile and run wasm-wasi
    let wasm = compile_and_run_wasm_wasi(&td_path, &wasmtime, &["--dir=."])
        .expect("wasm-wasi should succeed");

    let wasm_lines: Vec<&str> = wasm.lines().collect();
    assert!(
        wasm_lines.len() >= 2,
        "wasm output too short: {:?}",
        wasm_lines
    );

    // Line 1: isError() should be true
    assert_eq!(
        wasm_lines[0], "true",
        "writeFile to non-existent dir should report isError() = true"
    );

    // Line 2: toString() should preserve the inner error pack field names.
    assert!(
        wasm_lines[1].contains("type <=")
            && wasm_lines[1].contains("message <=")
            && wasm_lines[1].contains("kind <="),
        "Result toString should preserve error field names, got: {}",
        wasm_lines[1]
    );
}

/// Test: wasm-min non-regression — ensure wasm-min still works after wasm-wasi additions.
#[test]
fn wasm_wasi_does_not_break_wasm_min() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");

    // Just verify wasm-min compilation still works
    let wasm_path = std::env::temp_dir().join("taida_wasm_wasi_nonreg_hello.wasm");
    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");

    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        compile.status.success(),
        "wasm-min should still compile after wasm-wasi additions: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
}

// ---------------------------------------------------------------------------
// WW-3: Validation — parity, superset property, size checks
// ---------------------------------------------------------------------------

/// Run a .td file with the native backend and return its stdout.
fn run_native(td_path: &Path) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let native_path = std::env::temp_dir().join(format!("taida_native_wasi_test_{}", stem));

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

/// Compile a .td file to a given target and return the .wasm file size in bytes.
fn compile_wasm_and_get_size(td_path: &Path, target: &str, wasm_path: &Path) -> Option<u64> {
    let output = Command::new(taida_bin())
        .args(["build", "--target", target])
        .arg(td_path)
        .arg("-o")
        .arg(wasm_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let size = std::fs::metadata(wasm_path).map(|m| m.len()).ok()?;
    let _ = std::fs::remove_file(wasm_path);
    Some(size)
}

/// Compile a .td file to wasm-wasi and run with wasmtime (unique temp path for superset test).
fn compile_and_run_wasm_wasi_superset(td_path: &Path, wasmtime: &Path) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let wasm_path = std::env::temp_dir().join(format!("taida_ww3_superset_wasi_{}.wasm", stem));

    let compile_output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-wasi"])
        .arg(td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .ok()?;

    if !compile_output.status.success() {
        let stderr = String::from_utf8_lossy(&compile_output.stderr);
        eprintln!(
            "wasm-wasi compile failed for {}: {}",
            td_path.display(),
            stderr
        );
        return None;
    }

    let run_output = Command::new(wasmtime).arg(&wasm_path).output().ok()?;
    let _ = std::fs::remove_file(&wasm_path);

    if !run_output.status.success() {
        return None;
    }

    Some(
        String::from_utf8_lossy(&run_output.stdout)
            .trim_end()
            .to_string(),
    )
}

/// Compile a .td file to wasm-min and run with wasmtime.
fn compile_and_run_wasm_min(td_path: &Path, wasmtime: &Path) -> Option<String> {
    let stem = td_path.file_stem()?.to_string_lossy().to_string();
    let wasm_path = std::env::temp_dir().join(format!("taida_ww3_superset_min_{}.wasm", stem));

    let compile_output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .ok()?;

    if !compile_output.status.success() {
        return None;
    }

    let run_output = Command::new(wasmtime).arg(&wasm_path).output().ok()?;
    let _ = std::fs::remove_file(&wasm_path);

    if !run_output.status.success() {
        return None;
    }

    Some(
        String::from_utf8_lossy(&run_output.stdout)
            .trim_end()
            .to_string(),
    )
}

/// WW-3a: Comprehensive parity test for wasm-wasi.
/// For every .td file in examples/ that successfully compiles with wasm-wasi,
/// the wasm output must match the native backend output exactly.
/// Excludes wasm_wasi_env.td (needs --env args) and file I/O tests (need --dir).
#[test]
fn wasm_wasi_parity_all_examples() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi parity test");
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

    // Skip files that need special wasmtime args (env injection, dir access)
    // Also skip wasm_edge_* examples (different profile, tested in wasm_edge.rs)
    let skip_stems: Vec<&str> = vec![
        "wasm_wasi_env",                 // needs --env
        "wasm_wasi_file_io",             // needs --dir, creates temp files
        "wasm_wasi_exists",              // needs --dir, creates temp files
        "wasm_wasi_write_failure",       // needs --dir
        "wasm_wasi_write_failure_shape", // needs --dir
        "wasm_edge_env",                 // wasm-edge profile, needs taida_host imports
    ];

    let mut parity_ok = Vec::new();
    let mut parity_fail = Vec::new();
    let mut compile_rejected = Vec::new();
    let mut native_fail = Vec::new();

    for td_path in &td_files {
        let stem = td_path.file_stem().unwrap().to_string_lossy().to_string();

        // Skip files that need special runtime args
        if skip_stems.contains(&stem.as_str()) {
            continue;
        }

        // Try native build first
        let native_output = run_native(td_path);
        if native_output.is_none() {
            native_fail.push(stem.clone());
            continue;
        }
        let native_out = native_output.unwrap();

        // Try wasm-wasi compile + run (unique temp path to avoid collision with other tests)
        let parity_wasm_path = std::env::temp_dir().join(format!("taida_ww3_parity_{}.wasm", stem));
        let compile_output = Command::new(taida_bin())
            .args(["build", "--target", "wasm-wasi"])
            .arg(td_path)
            .arg("-o")
            .arg(&parity_wasm_path)
            .output()
            .ok();
        let wasm_output = compile_output.and_then(|co| {
            if !co.status.success() {
                return None;
            }
            let run = Command::new(&wasmtime)
                .arg(&parity_wasm_path)
                .output()
                .ok()?;
            let _ = std::fs::remove_file(&parity_wasm_path);
            if !run.status.success() {
                return None;
            }
            Some(String::from_utf8_lossy(&run.stdout).trim_end().to_string())
        });
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
        "WW-3 Parity: {} OK, {} rejected, {} native-fail",
        parity_ok.len(),
        compile_rejected.len(),
        native_fail.len()
    );

    if !parity_fail.is_empty() {
        let mut msg = format!("WW-3 PARITY FAILED for {} example(s):\n", parity_fail.len());
        for (stem, native, wasm) in &parity_fail {
            msg.push_str(&format!(
                "\n  {}: native='{}' vs wasm-wasi='{}'\n",
                stem,
                native.chars().take(100).collect::<String>(),
                wasm.chars().take(100).collect::<String>()
            ));
        }
        panic!("{}", msg);
    }

    // --- Strict regression guards (exact counts, not thresholds) ---

    // Expected allowlist: examples that wasm-wasi cannot compile (unsupported features).
    // If this list shrinks, update the count — that's progress.
    // If it grows, the test fails — that's a regression.
    // NTH-6: allowlist reduced after NTH-5 poly_add string support enabled
    // 3 examples (07_closures, compile_lambda, wasm_min_pi_approx) now pass parity.
    // WC-3: updated after list molds moved to core. Many list examples now compile.
    // Removed: 10_list_operations, 11_introspection, 16_unmold_both_directions,
    //          compile_hof_molds, compile_list, compile_list_map, compile_list_molds,
    //          compile_rc, compile_str_molds, compile_num_molds, todo_app
    let expected_rejected: Vec<&str> = vec![
        "06_lists",
        "09_modules",
        "13_async",
        "14_unmold_backward",
        "17_gorillax_cage",
        "18_std_json",
        "26_prelude_optional", // typeof works but taida_polymorphic_has_value missing
        "27_prelude_result",
        "28_prelude_collections",
        "30_class_like_methods",
        "api_client",
        "compile_async",
        "compile_gorillax",
        "compile_hashmap_set",
        "compile_json",
        "compile_lax",
        "compile_methods",
        "compile_module",
        "compile_module_value",
        "compile_optional_result",
        "compile_pack_field_call",
        "compile_prelude",
        "compile_type_conv",
    ];

    // Expected allowlist: examples where native backend itself fails.
    let expected_native_fail: Vec<&str> = vec![
        "compile_stream",
        "helper_val",
        "module_math",
        "module_utils",
        "transpile_npm",
    ];

    // Detect regressions: any new rejected/native-fail example not in the allowlist
    let unexpected_rejected: Vec<&String> = compile_rejected
        .iter()
        .filter(|s| !expected_rejected.contains(&s.as_str()))
        .collect();
    assert!(
        unexpected_rejected.is_empty(),
        "WW-3 REGRESSION: unexpected compile_rejected examples: {:?}",
        unexpected_rejected
    );

    let unexpected_native_fail: Vec<&String> = native_fail
        .iter()
        .filter(|s| !expected_native_fail.contains(&s.as_str()))
        .collect();
    assert!(
        unexpected_native_fail.is_empty(),
        "WW-3 REGRESSION: unexpected native_fail examples: {:?}",
        unexpected_native_fail
    );

    // Exact parity count — if this changes, update deliberately.
    // WE-2: wasm_edge_hello.td added (simple stdout, compilable by wasm-wasi too)
    // NTH-6: updated from 24 to 27 after NTH-5 poly_add string support
    // WC-1: compile_str_molds now passes on wasm-wasi (string molds in core) → 28
    // WC-2: compile_num_molds now passes on wasm-wasi (number molds in core) → 29
    // WC-3: list molds in core → 38 (compile_list_molds, compile_list_map, compile_hof_molds,
    //        todo_app, 10_list_operations, plus others now compile with list ops in core)
    assert_eq!(
        parity_ok.len(),
        38,
        "WW-3: Expected exactly 38 parity-OK examples, got {}. \
         If parity improved, update the expected count. List: {:?}",
        parity_ok.len(),
        parity_ok
    );

    // Guard against allowlist growing (regressions)
    assert!(
        compile_rejected.len() <= expected_rejected.len(),
        "WW-3: compile_rejected count ({}) exceeds expected allowlist ({}). \
         A previously compilable example regressed.",
        compile_rejected.len(),
        expected_rejected.len()
    );

    assert!(
        native_fail.len() <= expected_native_fail.len(),
        "WW-3: native_fail count ({}) exceeds expected allowlist ({}). \
         A previously working native example regressed.",
        native_fail.len(),
        expected_native_fail.len()
    );
}

/// WW-3b: Superset property — everything wasm-min can compile and run,
/// wasm-wasi must also compile, run, and produce identical output.
#[test]
fn wasm_wasi_superset_of_wasm_min() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-wasi superset test");
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

    let mut superset_ok = 0;
    let mut superset_fail = Vec::new();
    let mut min_only = Vec::new();

    for td_path in &td_files {
        let stem = td_path.file_stem().unwrap().to_string_lossy().to_string();

        // Skip wasm_wasi_* examples (they use OS APIs not in wasm-min)
        if stem.starts_with("wasm_wasi_") {
            continue;
        }

        // Try wasm-min first
        let min_output = compile_and_run_wasm_min(td_path, &wasmtime);
        if min_output.is_none() {
            // wasm-min cannot compile this — no superset obligation
            continue;
        }
        let min_out = min_output.unwrap();

        // wasm-wasi MUST also succeed (use superset-specific temp paths to avoid collision)
        let wasi_output = compile_and_run_wasm_wasi_superset(td_path, &wasmtime);
        if wasi_output.is_none() {
            min_only.push(stem.clone());
            continue;
        }
        let wasi_out = wasi_output.unwrap();

        if min_out == wasi_out {
            superset_ok += 1;
        } else {
            superset_fail.push((stem.clone(), min_out, wasi_out));
        }
    }

    eprintln!(
        "WW-3 Superset: {} OK, {} wasm-min-only (superset violation)",
        superset_ok,
        min_only.len()
    );

    // Superset violations: wasm-min succeeded but wasm-wasi failed
    assert!(
        min_only.is_empty(),
        "WW-3 SUPERSET VIOLATION: wasm-min succeeded but wasm-wasi failed for: {:?}",
        min_only
    );

    // Output mismatch: wasm-min and wasm-wasi produce different output
    if !superset_fail.is_empty() {
        let mut msg = format!(
            "WW-3 SUPERSET OUTPUT MISMATCH for {} example(s):\n",
            superset_fail.len()
        );
        for (stem, min_out, wasi_out) in &superset_fail {
            msg.push_str(&format!(
                "\n  {}: wasm-min='{}' vs wasm-wasi='{}'\n",
                stem,
                min_out.chars().take(100).collect::<String>(),
                wasi_out.chars().take(100).collect::<String>()
            ));
        }
        panic!("{}", msg);
    }

    // Exact superset count — if this changes, update deliberately.
    // WE-2: wasm_edge_hello.td added (simple stdout, compilable by both wasm-min and wasm-wasi)
    // NTH-6: updated from 23 to 26 after NTH-5 poly_add string support
    // WC-1: compile_str_molds now passes on wasm-wasi (string molds in core) → 27
    // WC-2: compile_num_molds now passes on wasm-wasi (number molds in core) → 28
    // WC-3: list molds in core → 37 (list operations now compile in both profiles)
    assert_eq!(
        superset_ok, 37,
        "WW-3: Expected exactly 37 superset-verified examples, got {}. \
         If superset coverage improved, update the expected count.",
        superset_ok
    );
}

/// WW-3c: Size check — wasm-wasi binaries should be reasonably bounded.
/// wasm-wasi includes WASI I/O layer, so binaries are larger than wasm-min,
/// but should not be excessively large.
#[test]
fn wasm_wasi_size_check() {
    let hello_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let stderr_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_stderr.td");

    let hello_min_size = compile_wasm_and_get_size(
        &hello_path,
        "wasm-min",
        &std::env::temp_dir().join("taida_ww3_hello_min.wasm"),
    )
    .expect("wasm-min hello should compile");

    let hello_wasi_size = compile_wasm_and_get_size(
        &hello_path,
        "wasm-wasi",
        &std::env::temp_dir().join("taida_ww3_hello_wasi.wasm"),
    )
    .expect("wasm-wasi hello should compile");

    let stderr_wasi_size = compile_wasm_and_get_size(
        &stderr_path,
        "wasm-wasi",
        &std::env::temp_dir().join("taida_ww3_stderr_wasi.wasm"),
    )
    .expect("wasm-wasi stderr should compile");

    eprintln!(
        "WW-3 Size: hello(wasm-min)={}B, hello(wasm-wasi)={}B, stderr(wasm-wasi)={}B",
        hello_min_size, hello_wasi_size, stderr_wasi_size
    );

    // wasm-wasi hello should be <= 4KB (WASI layer adds overhead but --gc-sections prunes)
    assert!(
        hello_wasi_size <= 4096,
        "WW-3: wasm-wasi hello should be <= 4KB, got {} bytes",
        hello_wasi_size
    );

    // wasm-wasi stderr should be <= 4KB (adds only fd_write to fd=2)
    assert!(
        stderr_wasi_size <= 4096,
        "WW-3: wasm-wasi stderr should be <= 4KB, got {} bytes",
        stderr_wasi_size
    );

    // wasm-wasi should not be more than 10x larger than wasm-min for the same code
    let ratio = hello_wasi_size as f64 / hello_min_size as f64;
    eprintln!("WW-3 Size ratio (wasm-wasi/wasm-min): {:.1}x", ratio);
    assert!(
        ratio <= 10.0,
        "WW-3: wasm-wasi should not be more than 10x larger than wasm-min, got {:.1}x",
        ratio
    );
}
