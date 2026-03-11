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
