mod common;

use common::{normalize, taida_bin, wasmtime_bin};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "registry_signature_parity_{}_{}_{}",
        label,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("mkdir fixture");
    dir
}

fn write_source(label: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = unique_dir(label);
    let td = dir.join("main.td");
    fs::write(&td, source).expect("write source");
    (dir, td)
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cc_available() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn interpreter_error(td: &Path) -> Option<String> {
    let output = Command::new(taida_bin()).arg(td).output().ok()?;
    if output.status.success() {
        return None;
    }
    Some(normalize(&String::from_utf8_lossy(&output.stderr)))
}

fn build_error(td: &Path, dir: &Path, target: &str) -> Option<String> {
    let out_path = dir.join(match target {
        "js" => "out.mjs",
        "native" => "out.bin",
        "wasm-full" => "out.wasm",
        _ => "out.artifact",
    });
    let output = Command::new(taida_bin())
        .args(["build", target])
        .arg(td)
        .arg("-o")
        .arg(&out_path)
        .output()
        .ok()?;
    let _ = fs::remove_file(&out_path);
    if output.status.success() {
        return None;
    }
    Some(normalize(&String::from_utf8_lossy(&output.stderr)))
}

fn assert_all_backend_paths_reject(source: &str, label: &str, expected: &str) {
    let (dir, td) = write_source(label, source);
    let backends = [
        ("interpreter", interpreter_error(&td)),
        ("js", build_error(&td, &dir, "js")),
        ("native", build_error(&td, &dir, "native")),
        ("wasm-full", build_error(&td, &dir, "wasm-full")),
    ];
    let _ = fs::remove_dir_all(&dir);

    for (backend, err) in backends {
        let err = err.unwrap_or_else(|| panic!("{label}: {backend} unexpectedly accepted source"));
        assert!(
            err.contains(expected),
            "{label}: {backend} error should contain {expected}, got: {err}"
        );
    }
}

fn run_interpreter(td: &Path) -> Result<String, String> {
    let output = Command::new(taida_bin())
        .arg(td)
        .output()
        .map_err(|e| format!("spawn interpreter: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(normalize(&String::from_utf8_lossy(&output.stdout)))
}

fn run_js(td: &Path, dir: &Path) -> Result<Option<String>, String> {
    if !node_available() {
        eprintln!("node unavailable; skipping JS leg");
        return Ok(None);
    }
    let out_path = dir.join("out.mjs");
    let build = Command::new(taida_bin())
        .args(["build", "js"])
        .arg(td)
        .arg("-o")
        .arg(&out_path)
        .output()
        .map_err(|e| format!("spawn js build: {e}"))?;
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).to_string());
    }
    let run = Command::new("node")
        .arg(&out_path)
        .output()
        .map_err(|e| format!("spawn node: {e}"))?;
    if !run.status.success() {
        return Err(String::from_utf8_lossy(&run.stderr).to_string());
    }
    Ok(Some(normalize(&String::from_utf8_lossy(&run.stdout))))
}

fn run_native(td: &Path, dir: &Path) -> Result<Option<String>, String> {
    if !cc_available() {
        eprintln!("cc unavailable; skipping native leg");
        return Ok(None);
    }
    let out_path = dir.join("out.bin");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&out_path)
        .output()
        .map_err(|e| format!("spawn native build: {e}"))?;
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).to_string());
    }
    let run = Command::new(&out_path)
        .output()
        .map_err(|e| format!("spawn native binary: {e}"))?;
    if !run.status.success() {
        return Err(String::from_utf8_lossy(&run.stderr).to_string());
    }
    Ok(Some(normalize(&String::from_utf8_lossy(&run.stdout))))
}

fn run_wasm_full(td: &Path, dir: &Path) -> Result<Option<String>, String> {
    let Some(wasmtime) = wasmtime_bin() else {
        eprintln!("wasmtime unavailable; skipping wasm-full leg");
        return Ok(None);
    };
    let out_path = dir.join("out.wasm");
    let build = Command::new(taida_bin())
        .args(["build", "wasm-full"])
        .arg(td)
        .arg("-o")
        .arg(&out_path)
        .output()
        .map_err(|e| format!("spawn wasm-full build: {e}"))?;
    if !build.status.success() {
        return Err(String::from_utf8_lossy(&build.stderr).to_string());
    }
    let run = Command::new(wasmtime)
        .args(["run", "--"])
        .arg(&out_path)
        .output()
        .map_err(|e| format!("spawn wasmtime: {e}"))?;
    if !run.status.success() {
        return Err(String::from_utf8_lossy(&run.stderr).to_string());
    }
    Ok(Some(normalize(&String::from_utf8_lossy(&run.stdout))))
}

fn assert_runtime_parity(source: &str, label: &str, expected: &str) {
    let (dir, td) = write_source(label, source);
    let interp = run_interpreter(&td).expect("interpreter run");
    assert_eq!(interp, expected, "{label}: interpreter output mismatch");

    if let Some(js) = run_js(&td, &dir).expect("js run") {
        assert_eq!(js, interp, "{label}: js output mismatch");
    }
    if let Some(native) = run_native(&td, &dir).expect("native run") {
        assert_eq!(native, interp, "{label}: native output mismatch");
    }
    if let Some(wasm) = run_wasm_full(&td, &dir).expect("wasm-full run") {
        assert_eq!(wasm, interp, "{label}: wasm-full output mismatch");
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn builtin_mold_registry_signature_rejects_on_all_build_paths() {
    let cases = [
        (
            "map_missing_callback",
            r#"
nums <= @[1, 2]
out <= Map[nums]()
"#,
            "[E1505]",
        ),
        (
            "filter_non_function",
            r#"
nums <= @[1, 2]
out <= Filter[nums, 1]()
"#,
            "[E1506]",
        ),
        (
            "filter_non_bool_callback",
            r#"
bad n: Int = n + 1 => :Int
nums <= @[1, 2]
out <= Filter[nums, bad]()
"#,
            "[E1506]",
        ),
        (
            "fold_unary_callback",
            r#"
step acc: Int = acc + 1 => :Int
nums <= @[1, 2]
out <= Fold[nums, 0, step]()
"#,
            "[E1506]",
        ),
        (
            "sort_unknown_option",
            r#"
xs <= @[3, 1, 2]
out <= Sort[xs](bogus <= true)
"#,
            "[E1406]",
        ),
        (
            "sort_by_non_function",
            r#"
xs <= @[3, 1, 2]
out <= Sort[xs](by <= 1)
"#,
            "[E1506]",
        ),
    ];

    for (label, source, expected) in cases {
        assert_all_backend_paths_reject(source, label, expected);
    }
}

#[test]
fn builtin_mold_registry_callback_options_run_across_backends() {
    let source = r#"
keyDesc n: Int = 0 - n => :Int
parity n: Int = Mod[n, 2]().getOrDefault(0) => :Int
xs <= @[3, 1, 2]
stdout(Join[Sort[xs](by <= keyDesc), ","]())
stdout(Join[Unique[@[1, 3, 2, 4, 5]](by <= parity), ","]())
stdout(Join[Sort[xs](desc <= true), ","]())
"#;
    assert_runtime_parity(source, "registry_callback_options", "3,2,1\n1,2\n3,2,1");
}
