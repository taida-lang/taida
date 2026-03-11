/// Integration tests for wasm-edge backend.
///
/// WE-2/WE-3: Validates that wasm-edge compiles correctly,
/// rejects unsupported APIs, and does not regress wasm-min/wasm-wasi.
///
/// Note: wasm-edge outputs require a JS glue host (or wasmtime with
/// taida_host imports provided). For compile-only tests we just verify
/// the compilation succeeds or fails as expected. For runtime tests,
/// we use wasmtime (which provides wasi_snapshot_preview1.fd_write)
/// for the basic stdout path -- the wasm-edge hello example only needs
/// fd_write and does NOT use taida_host imports.
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

/// Compile a .td file with wasm-edge and return the wasm path (or None on failure).
fn compile_wasm_edge(td_path: &Path, wasm_path: &Path) -> Option<String> {
    let output = Command::new(taida_bin())
        .args(["build", "--target", "wasm-edge"])
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
// WE-3a: Smoke tests
// ---------------------------------------------------------------------------

/// Test: wasm-edge compiles the hello example.
/// The resulting .wasm only uses wasi_snapshot_preview1.fd_write (no taida_host),
/// so wasmtime can run it directly.
#[test]
fn wasm_edge_hello_compiles() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_edge_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_test_hello.wasm");

    let err = compile_wasm_edge(&td_path, &wasm_path);
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        err.is_none(),
        "wasm-edge hello should compile, got: {:?}",
        err
    );
}

/// Test: wasm-edge hello produces correct output when run with wasmtime.
/// wasmtime provides wasi_snapshot_preview1.fd_write which is all hello needs.
#[test]
fn wasm_edge_hello_runs() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-edge runtime test");
            return;
        }
    };

    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_edge_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_test_hello_run.wasm");

    let err = compile_wasm_edge(&td_path, &wasm_path);
    assert!(err.is_none(), "compile failed: {:?}", err);

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
    assert_eq!(stdout, "Hello from edge!");
}

/// Test: wasm-edge env example compiles (runtime test skipped -- needs taida_host imports).
#[test]
fn wasm_edge_env_compiles() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_edge_env.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_test_env.wasm");

    let err = compile_wasm_edge(&td_path, &wasm_path);
    let _ = std::fs::remove_file(&wasm_path);

    assert!(
        err.is_none(),
        "wasm-edge env should compile, got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// WE-3b: Negative tests
// ---------------------------------------------------------------------------

/// Test: wasm-edge rejects file I/O APIs with clear error message.
#[test]
fn wasm_edge_rejects_file_io() {
    let td_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_wasi_file_io.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_test_file_reject.wasm");

    let err = compile_wasm_edge(&td_path, &wasm_path);
    let _ = std::fs::remove_file(&wasm_path);

    assert!(err.is_some(), "wasm-edge should reject file I/O");
    let msg = err.unwrap();
    assert!(
        msg.contains("wasm-edge does not support"),
        "error should mention wasm-edge, got: {}",
        msg
    );
}

// ---------------------------------------------------------------------------
// WE-3c: Non-regression
// ---------------------------------------------------------------------------

/// Test: wasm-min still works after wasm-edge additions.
#[test]
fn wasm_edge_does_not_break_wasm_min() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_nonreg_min.wasm");

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
        "wasm-min should still compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
}

/// Test: wasm-wasi still works after wasm-edge additions.
#[test]
fn wasm_edge_does_not_break_wasm_wasi() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_nonreg_wasi.wasm");

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
        "wasm-wasi should still compile: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
}

/// Test: wasm-edge binary size is bounded.
#[test]
fn wasm_edge_size_check() {
    let td_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_edge_hello.td");
    let wasm_path = std::env::temp_dir().join("taida_wasm_edge_size_hello.wasm");

    let compile = Command::new(taida_bin())
        .args(["build", "--target", "wasm-edge"])
        .arg(&td_path)
        .arg("-o")
        .arg(&wasm_path)
        .output()
        .expect("compile should run");
    assert!(
        compile.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let size = std::fs::metadata(&wasm_path).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_file(&wasm_path);

    eprintln!("WE-3: wasm-edge hello size = {} bytes", size);

    // wasm-edge hello should be small (same as wasm-min -- no extra imports GC'd)
    assert!(
        size <= 4096,
        "wasm-edge hello should be <= 4KB, got {} bytes",
        size
    );
}
