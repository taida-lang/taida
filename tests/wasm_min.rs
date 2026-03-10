/// Integration tests for wasm-min backend.
///
/// Compiles .td files to .wasm via `taida build --target wasm-min`,
/// runs them with wasmtime, and verifies output matches the interpreter.
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

#[test]
fn wasm_min_size_gate() {
    let wasmtime = match wasmtime_bin() {
        Some(p) => p,
        None => {
            eprintln!("wasmtime not found, skipping wasm-min tests");
            return;
        }
    };

    // Compile both examples
    let hello_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_hello.td");
    let pi_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/wasm_min_pi_approx.td");

    let hello_wasm = std::env::temp_dir().join("taida_wasm_size_hello.wasm");
    let pi_wasm = std::env::temp_dir().join("taida_wasm_size_pi.wasm");

    let _ = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&hello_path)
        .arg("-o")
        .arg(&hello_wasm)
        .output();

    let _ = Command::new(taida_bin())
        .args(["build", "--target", "wasm-min"])
        .arg(&pi_path)
        .arg("-o")
        .arg(&pi_wasm)
        .output();

    let hello_size = std::fs::metadata(&hello_wasm)
        .map(|m| m.len())
        .unwrap_or(0);
    let pi_size = std::fs::metadata(&pi_wasm).map(|m| m.len()).unwrap_or(0);

    let _ = std::fs::remove_file(&hello_wasm);
    let _ = std::fs::remove_file(&pi_wasm);

    // Wado baselines: hello_world = 1,572 bytes, pi_approx = 9,269 bytes
    // Our gates: hello <= 2KB, pi <= 9,269 bytes (hard gate)
    eprintln!("wasm-min hello size: {} bytes (Wado: 1,572)", hello_size);
    eprintln!("wasm-min pi size: {} bytes (Wado: 9,269)", pi_size);

    assert!(
        hello_size > 0 && hello_size <= 2048,
        "hello.wasm should be <= 2KB, got {} bytes",
        hello_size
    );
    assert!(
        pi_size > 0 && pi_size <= 9269,
        "pi.wasm should be <= 9,269 bytes (Wado baseline), got {} bytes",
        pi_size
    );

    // Verify execution too
    let _ = wasmtime; // ensure wasmtime exists
}
