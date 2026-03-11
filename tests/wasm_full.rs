/// Integration tests for wasm-full backend.
///
/// WF-2a: Validates that wasm-full compiles correctly,
/// does not regress wasm-min/wasm-wasi, and produces correct output.
///
/// wasm-full is a superset of wasm-wasi, which is a superset of wasm-min.
/// It adds extended runtime functions (collections, string/number molds,
/// JSON, Gorillax, bytes, bitwise, etc.) on top of wasm-wasi.
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
    if let Ok(output) = Command::new("which").arg("wasmtime").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Compile a .td file with wasm-full and return the error message (or None on success).
fn compile_wasm_full(td_path: &Path, wasm_path: &Path) -> Option<String> {
    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-full"])
        .arg(td_path)
        .arg("-o")
        .arg(wasm_path)
        .output()
        .ok()?;

    if output.status.success() {
        None // no error
    } else {
        Some(
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .to_string(),
        )
    }
}

// ---------------------------------------------------------------------------
// WF-2a: Smoke tests
// ---------------------------------------------------------------------------

/// Test: wasm-full compiles the hello example and produces correct output.
/// wasm-full is a superset of wasm-wasi, so wasm_min_hello.td should work.
#[test]
fn wasm_full_hello() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full hello test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_test_hello.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full hello should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();
    assert_eq!(stdout, "hello");
}

/// Test: wasm-full compiles and runs the env example via wasmtime --env.
/// wasm-full inherits wasm-wasi's env support.
#[test]
fn wasm_full_env() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full env test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_env.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_test_env.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full env should compile, got: {:?}",
        err
    );

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

    let stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() >= 2, "wasm-full env output too short: {:?}", lines);
    assert_eq!(lines[0], "hello", "EnvVar should resolve TAIDA_TEST_A=hello");
    assert_eq!(lines[1], "2", "allEnv should see exactly 2 injected env vars");
}

// ---------------------------------------------------------------------------
// WF-2b: String molds parity test
// ---------------------------------------------------------------------------

/// Test: wasm-full compiles and runs compile_str_molds.td with native-parity output.
/// This validates all string mold functions implemented in runtime_full_wasm.c:
/// Upper, Lower, Trim, Replace (first and all), CharAt, Repeat, Reverse, Slice, Pad.
#[test]
fn wasm_full_str_molds_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full str_molds parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_str_molds.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_str_molds.wasm");

    // Compile with wasm-full
    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full str_molds should compile, got: {:?}",
        err
    );

    // Run with wasmtime
    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    // Get native output for parity comparison
    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full str_molds output should match native/interpreter output"
    );
}

/// Test: wasm-full compiles and runs compile_num_molds.td with native-parity output.
/// This validates all numeric mold functions implemented in runtime_full_wasm.c:
/// ToFixed, Abs (Int), Floor, Ceil, Round, Truncate, Clamp (Int).
#[test]
fn wasm_full_num_molds_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full num_molds parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_num_molds.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_num_molds.wasm");

    // Compile with wasm-full
    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full num_molds should compile, got: {:?}",
        err
    );

    // Run with wasmtime
    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    // Get native output for parity comparison
    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full num_molds output should match native/interpreter output"
    );
}

/// Test: wasm-full compiles and runs compile_list_molds.td with native-parity output.
/// This validates extended list operations in runtime_full_wasm.c:
/// Join, Sum, Concat, Append, Prepend, Sort, Sort(reverse), Unique, Flatten, FindIndex, Count.
#[test]
fn wasm_full_list_molds_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full list_molds parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_list_molds.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_list_molds.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full list_molds should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full list_molds output should match native/interpreter output"
    );
}

/// Test: wasm-full compiles and runs compile_list_map.td with native-parity output.
#[test]
fn wasm_full_list_map_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full list_map parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_list_map.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_list_map.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full list_map should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full list_map output should match native/interpreter output"
    );
}

/// Test: wasm-full compiles and runs compile_hashmap_set.td with native-parity output.
#[test]
fn wasm_full_hashmap_set_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full hashmap_set parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_hashmap_set.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_hashmap_set.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full hashmap_set should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full hashmap_set output should match native/interpreter output"
    );
}

// ---------------------------------------------------------------------------
// WF-2f: Polymorphic dispatch + Lax/Result parity tests
// ---------------------------------------------------------------------------

/// Test: wasm-full compile_methods.td matches interpreter output.
/// Validates: string contains/indexOf/lastIndexOf on static strings (low-address data section),
/// list.get() returning Lax, list.hasValue() on Lax, and all other state-check methods.
#[test]
fn wasm_full_methods_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full methods parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_methods.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_methods.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full methods should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full methods output should match interpreter output"
    );
}

/// Test: wasm-full compile_optional_result.td matches interpreter output.
/// Validates: Lax.isEmpty(), Lax.hasValue(), Div[0,0]().isEmpty() = true,
/// Result methods (isSuccess, isError, getOrDefault, map, flatMap).
#[test]
fn wasm_full_optional_result_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full optional_result parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_optional_result.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_optional_result.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full optional_result should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full optional_result output should match interpreter output"
    );
}

// ---------------------------------------------------------------------------
// WF-3a: JSON runtime parity tests
// ---------------------------------------------------------------------------

/// Test: wasm-full compiles and runs compile_json.td with native-parity output.
/// Validates JSON[raw, Schema]() schema cast with all rules.
#[test]
fn wasm_full_json_schema_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full json_schema parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_json.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_json_schema.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full json_schema should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full json_schema output should match interpreter output"
    );
}

/// Test: wasm-full compiles and runs compile_prelude.td with native-parity output.
/// Validates jsonEncode/jsonPretty serialization.
#[test]
fn wasm_full_json_encode_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full json_encode parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compile_prelude.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_json_encode.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full json_encode should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full json_encode output should match interpreter output"
    );
}

/// Test: wasm-full compiles and runs 18_std_json.td with native-parity output.
/// Validates full JSON workflow: schema cast + encode + pretty + nested + list.
#[test]
fn wasm_full_json_full_parity() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full json_full parity test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/18_std_json.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_json_full.wasm");

    let err = compile_wasm_full(&td_path, &wasm_path);
    assert!(
        err.is_none(),
        "wasm-full json_full should compile, got: {:?}",
        err
    );

    let run = Command::new(&wasmtime)
        .arg("run")
        .arg("--")
        .arg(&wasm_path)
        .output()
        .expect("wasmtime should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        run.status.success(),
        "wasmtime failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let wasm_stdout = String::from_utf8_lossy(&run.stdout).trim_end().to_string();

    let native_run = Command::new(taida_bin())
        .arg(&td_path)
        .output()
        .expect("interpreter should run");
    let native_stdout = String::from_utf8_lossy(&native_run.stdout)
        .trim_end()
        .to_string();

    assert_eq!(
        wasm_stdout, native_stdout,
        "wasm-full json_full output should match interpreter output"
    );
}

// ---------------------------------------------------------------------------
// WF-5a: Comprehensive parity test
// ---------------------------------------------------------------------------

/// Run native binary for a .td file, return stdout or None on failure.
fn run_native(td_path: &Path) -> Option<String> {
    let native_path = std::env::temp_dir().join(format!(
        "taida_wf5_native_{}",
        td_path.file_stem()?.to_string_lossy()
    ));
    let build = Command::new(taida_bin())
        .args(["build", "--target", "native"])
        .arg(td_path)
        .arg("-o")
        .arg(&native_path)
        .output()
        .ok()?;
    if !build.status.success() {
        return None;
    }
    let run = Command::new(&native_path).output().ok()?;
    let _ = std::fs::remove_file(&native_path);
    if !run.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).trim_end().to_string())
}

/// WF-5a: For every .td file that compiles with wasm-full,
/// the output must match the native backend output exactly.
#[test]
fn wasm_full_parity_all_examples() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-full parity test");
            return;
        }
    };

    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut td_files: Vec<_> = std::fs::read_dir(&examples_dir)
        .expect("examples/ directory should exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "td"))
        .collect();
    td_files.sort();

    // Skip: WASI tests needing --env/--dir, edge tests needing host imports
    let skip_stems: Vec<&str> = vec![
        "wasm_wasi_env",
        "wasm_wasi_exists",
        "wasm_wasi_file_io",
        "wasm_wasi_write_failure",
        "wasm_wasi_write_failure_shape",
        "wasm_wasi_stderr",         // stderr goes to separate fd
        "wasm_edge_env",            // edge profile, different env mechanism
    ];

    let mut parity_ok = Vec::new();
    let mut parity_fail = Vec::new();
    let mut compile_rejected = Vec::new();
    let mut native_fail = Vec::new();

    for td_path in &td_files {
        let stem = td_path.file_stem().unwrap().to_string_lossy().to_string();
        if skip_stems.contains(&stem.as_str()) {
            continue;
        }

        // Native build + run
        let native_output = run_native(td_path);
        if native_output.is_none() {
            native_fail.push(stem.clone());
            continue;
        }
        let native_out = native_output.unwrap();

        // wasm-full compile + run
        let wasm_path =
            std::env::temp_dir().join(format!("taida_wf5_parity_{}.wasm", stem));
        let compile_output = Command::new(taida_bin())
            .args(["build", "--target", "wasm-full"])
            .arg(td_path)
            .arg("-o")
            .arg(&wasm_path)
            .output()
            .ok();
        let wasm_output = compile_output.and_then(|co| {
            if !co.status.success() {
                return None;
            }
            let run = Command::new(&wasmtime)
                .arg("run")
                .arg("--")
                .arg(&wasm_path)
                .output()
                .ok()?;
            let _ = std::fs::remove_file(&wasm_path);
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

        // Known non-parity examples (pre-existing bugs in native/wasm, not wasm-full regressions)
        let known_mismatch: Vec<&str> = vec![
            "06_lists",          // string Reverse mold garbled on both backends differently
            "11_introspection",  // pointer addresses differ between memory layouts
            "27_prelude_result", // mapError toString differs (different error representation)
        ];

        if native_out == wasm_out {
            parity_ok.push(stem.clone());
        } else if known_mismatch.contains(&stem.as_str()) {
            // Expected mismatch, skip
        } else {
            parity_fail.push((stem.clone(), native_out, wasm_out));
        }
    }

    eprintln!(
        "WF-5 Parity: {} OK, {} rejected, {} native-fail",
        parity_ok.len(),
        compile_rejected.len(),
        native_fail.len()
    );

    if !parity_fail.is_empty() {
        let mut msg = format!(
            "WF-5 PARITY FAILED for {} example(s):\n",
            parity_fail.len()
        );
        for (stem, native, wasm) in &parity_fail {
            msg.push_str(&format!(
                "\n  {}: native='{}' vs wasm-full='{}'\n",
                stem,
                native.chars().take(100).collect::<String>(),
                wasm.chars().take(100).collect::<String>()
            ));
        }
        panic!("{}", msg);
    }

    // Expected compile_rejected: modules, async, native-only
    let expected_rejected: Vec<&str> = vec![
        "09_modules", "13_async", "14_unmold_backward",
        "api_client", "compile_async", "compile_module",
        "compile_module_value", "compile_stream",
        "helper_val", "module_math", "module_utils",
        "transpile_npm",
    ];

    // Expected native_fail (native build or run fails for these)
    let expected_native_fail: Vec<&str> = vec![
        "26_prelude_optional",
        "compile_stream",
        "helper_val",
        "module_math",
        "module_utils",
        "transpile_npm",
    ];

    // Detect regressions
    let unexpected_rejected: Vec<&String> = compile_rejected
        .iter()
        .filter(|s| !expected_rejected.contains(&s.as_str()))
        .collect();
    assert!(
        unexpected_rejected.is_empty(),
        "WF-5 REGRESSION: unexpected compile_rejected: {:?}",
        unexpected_rejected
    );

    let unexpected_native_fail: Vec<&String> = native_fail
        .iter()
        .filter(|s| !expected_native_fail.contains(&s.as_str()))
        .collect();
    assert!(
        unexpected_native_fail.is_empty(),
        "WF-5 REGRESSION: unexpected native_fail: {:?}",
        unexpected_native_fail
    );

    // Exact parity count: 49 examples match native output.
    // Known non-parity (excluded from this count):
    //   06_lists: string Reverse mold produces different garbled output on native vs wasm
    //   11_introspection: pointer addresses differ between native and wasm memory layouts
    //   27_prelude_result: mapError toString differs (different error representation)
    assert!(
        parity_ok.len() >= 49,
        "WF-5 REGRESSION: expected >= 49 parity, got {}. OK: {:?}",
        parity_ok.len(),
        parity_ok
    );
}

// ---------------------------------------------------------------------------
// Non-regression tests
// ---------------------------------------------------------------------------

/// Test: wasm-min still works after wasm-full additions.
#[test]
fn wasm_full_does_not_break_wasm_min() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_nonreg_min.wasm");

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
        "wasm-min should still compile after wasm-full additions: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
}

/// Test: wasm-wasi still works after wasm-full additions.
#[test]
fn wasm_full_does_not_break_wasm_wasi() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_full_nonreg_wasi.wasm");

    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-wasi"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        compile.status.success(),
        "wasm-wasi should still compile after wasm-full additions: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
}
