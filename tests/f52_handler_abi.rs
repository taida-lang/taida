#![cfg(feature = "native")]

mod common;

use common::{run_interpreter, taida_bin};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn compile_handler(target: &str, td_path: &Path, wasm_path: &Path) -> Option<String> {
    let output = Command::new(taida_bin())
        .args(["build", target, "--no-cache", "--handler", "handle"])
        .arg(td_path)
        .arg("-o")
        .arg(wasm_path)
        .output()
        .ok()?;

    if output.status.success() {
        None
    } else {
        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[test]
fn f52_interpreter_response_helpers_construct_meaningful_values() {
    let source = r#">>> taida-lang/abi => @(text, json, bytes, status, header)
>>> taida-lang/crypto => @(sha256)

textResponse <= header("x-mode", "interp", status(202, text("hello")))
stdout(textResponse.status.toString() + ":" + textResponse.headers.get("x-mode").getOrDefault("") + ":" + sha256(textResponse.body))

jsonResponse <= json(@(ok <= "yes"))
stdout(jsonResponse.headers.get("content-type").getOrDefault(""))

Bytes["Hi"]() >=> raw
bytesResponse <= bytes(raw)
stdout(bytesResponse.headers.get("content-type").getOrDefault("") + ":" + sha256(bytesResponse.body))
"#;

    let stem = format!("taida_f52_interp_{}", std::process::id());
    let td_path = std::env::temp_dir().join(format!("{}.td", stem));
    let _ = std::fs::remove_file(&td_path);
    std::fs::write(&td_path, source).expect("write interpreter F52 helper fixture");

    let out = run_interpreter(&td_path).expect("interpreter helper fixture should run");
    assert!(
        out.contains("202:interp:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"),
        "text/status/header helper output mismatch: {}",
        out
    );
    assert!(
        out.contains("application/json"),
        "json helper content-type missing: {}",
        out
    );
    assert!(
        out.contains(
            "application/octet-stream:3639efcd08abb273b1619e82e78c29a7df02c1051b1820e99fc395dcaa3326b8"
        ),
        "bytes helper output mismatch: {}",
        out
    );

    let _ = std::fs::remove_file(&td_path);
}

fn run_handler_with_node(wasm_path: &Path, label: &str) -> Option<String> {
    let js_path = std::env::temp_dir().join(format!(
        "taida_f52_handler_{}_{}.js",
        label,
        std::process::id()
    ));
    let wasm_for_js = wasm_path.to_string_lossy();
    let script = format!(
        r#"
const fs = require("fs");

(async () => {{
  let memory = new WebAssembly.Memory({{ initial: 2 }});
  const noOpWasi = new Proxy({{}}, {{
    get(_target, prop) {{
      if (prop === "fd_write") {{
        return (_fd, _iovsPtr, _iovsLen, nwrittenPtr) => {{
          new DataView(memory.buffer).setUint32(nwrittenPtr, 0, true);
          return 0;
        }};
      }}
      return () => 0;
    }},
  }});
  const imports = {{
    env: {{ memory }},
    wasi_snapshot_preview1: noOpWasi,
    taida_host: {{
      env_get() {{ return 0; }},
      env_get_all() {{ return 0; }},
    }},
  }};
  const wasm = fs.readFileSync("{wasm_for_js}");
  const {{ instance }} = await WebAssembly.instantiate(wasm, imports);
  if (instance.exports.memory) {{
    memory = instance.exports.memory;
  }}
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const payload = encoder.encode(JSON.stringify({{
    method: "POST",
    path: "/f52",
    query: {{ q: "search" }},
    headers: {{ "x-mode": "edge" }},
    bodyBase64: "cGF5bG9hZA==",
  }}));
  const inPtr = instance.exports.taida_abi_web_alloc(payload.length);
  new Uint8Array(memory.buffer, inPtr, payload.length).set(payload);
  const handle = instance.exports.taida_abi_web_handle(inPtr, payload.length);
  const outPtr = instance.exports.taida_abi_web_out_ptr(handle);
  const outLen = instance.exports.taida_abi_web_out_len(handle);
  const raw = decoder.decode(new Uint8Array(memory.buffer, outPtr, outLen));
  instance.exports.taida_abi_web_free(handle);
  const response = JSON.parse(raw);
  console.log(Buffer.from(response.bodyBase64, "base64").toString("utf8"));
}})().catch((err) => {{
  console.error(err && err.stack ? err.stack : err);
  process.exit(1);
}});
"#
    );
    std::fs::write(&js_path, script).ok()?;
    let output = Command::new("node").arg(&js_path).output().ok()?;
    let _ = std::fs::remove_file(&js_path);
    if !output.status.success() {
        eprintln!(
            "node handler host failed for {}: {}",
            label,
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_handler_native(bin_path: &Path, payload: &[u8]) -> Option<String> {
    let mut child = Command::new(bin_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        stdin.write_all(payload).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        eprintln!(
            "native handler host failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[test]
fn f52_wasm_profiles_share_handler_json_fixture() {
    if !node_available() {
        eprintln!("node not found, skipping F52 handler ABI smoke test");
        return;
    }

    let source = r#">>> taida-lang/abi => @(WebRequest, WebResponse, text)

handle req: WebRequest = text(req.method + ":" + req.path + ":" + req.query.get("q").getOrDefault("") + ":" + req.headers.get("x-mode").getOrDefault("")) => :WebResponse
"#;

    for target in ["wasm-min", "wasm-wasi", "wasm-full", "wasm-edge"] {
        let stem = format!(
            "taida_f52_{}_{}",
            target.replace('-', "_"),
            std::process::id()
        );
        let td_path = std::env::temp_dir().join(format!("{}.td", stem));
        let wasm_path: PathBuf = std::env::temp_dir().join(format!("{}.wasm", stem));
        let glue_path = std::env::temp_dir().join(format!("{}.edge.js", stem));

        let _ = std::fs::remove_file(&td_path);
        let _ = std::fs::remove_file(&wasm_path);
        let _ = std::fs::remove_file(&glue_path);
        std::fs::write(&td_path, source).expect("write F52 handler fixture");

        let err = compile_handler(target, &td_path, &wasm_path);
        assert!(
            err.is_none(),
            "{} handler build should compile: {:?}",
            target,
            err
        );
        let body = run_handler_with_node(&wasm_path, target)
            .unwrap_or_else(|| panic!("{} handler host should run", target));
        assert_eq!(body, "POST:/f52:search:edge", "target={}", target);

        let _ = std::fs::remove_file(&td_path);
        let _ = std::fs::remove_file(&wasm_path);
        let _ = std::fs::remove_file(&glue_path);
    }
}

#[test]
fn f52_native_handler_decodes_request_body_bytes() {
    let source = r#">>> taida-lang/abi => @(WebRequest, WebResponse, text)
>>> taida-lang/crypto => @(sha256)

handle req: WebRequest = text(req.method + ":" + req.path + ":" + req.query.get("q").getOrDefault("") + ":" + req.headers.get("x-mode").getOrDefault("") + ":" + sha256(req.body)) => :WebResponse
"#;

    let stem = format!("taida_f52_native_{}", std::process::id());
    let td_path = std::env::temp_dir().join(format!("{}.td", stem));
    let bin_path: PathBuf = std::env::temp_dir().join(stem);

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
    std::fs::write(&td_path, source).expect("write native F52 handler fixture");

    let err = compile_handler("native", &td_path, &bin_path);
    assert!(
        err.is_none(),
        "native handler build should compile: {:?}",
        err
    );
    let raw = run_handler_native(
        &bin_path,
        br#"{"method":"POST","path":"/f52","query":{"q":"search"},"headers":{"x-mode":"native"},"bodyBase64":"cGF5bG9hZA=="}"#,
    )
    .expect("native handler should run");
    assert!(
        raw.contains("\"status\":200"),
        "native handler response must report status 200: {}",
        raw
    );
    assert!(
        raw.contains(
            "\"bodyBase64\":\"UE9TVDovZjUyOnNlYXJjaDpuYXRpdmU6MjM5ZjU5ZWQ1NWU3MzdjNzcxNDdjZjU1YWQwYzFiMDMwYjZkN2VlNzQ4YTc0MjY5NTJmOWI4NTJkNWE5MzVlNQ==\""
        ),
        "native handler body must include decoded request fields and sha256(body): {}",
        raw
    );

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
}

#[test]
fn f52_native_handler_malformed_request_uses_default_shape() {
    let source = r#">>> taida-lang/abi => @(WebRequest, WebResponse, text)
>>> taida-lang/crypto => @(sha256)

handle req: WebRequest = text(req.method + ":" + req.path + ":" + sha256(req.body)) => :WebResponse
"#;

    let stem = format!("taida_f52_native_malformed_{}", std::process::id());
    let td_path = std::env::temp_dir().join(format!("{}.td", stem));
    let bin_path: PathBuf = std::env::temp_dir().join(stem);

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
    std::fs::write(&td_path, source).expect("write native malformed F52 handler fixture");

    let err = compile_handler("native", &td_path, &bin_path);
    assert!(
        err.is_none(),
        "native handler build should compile: {:?}",
        err
    );
    let raw = run_handler_native(&bin_path, b"not json").expect("native handler should run");
    assert!(
        raw.contains(
            "\"bodyBase64\":\"R0VUOi86ZTNiMGM0NDI5OGZjMWMxNDlhZmJmNGM4OTk2ZmI5MjQyN2FlNDFlNDY0OWI5MzRjYTQ5NTk5MWI3ODUyYjg1NQ==\""
        ),
        "malformed native request should use default method/path/body: {}",
        raw
    );

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
}

#[test]
fn f52_handler_mode_still_checks_surface_under_no_check() {
    let source = r#">>> taida-lang/abi => @(WebRequest, WebResponse)

handle req: WebRequest = @() => :WebResponse
"#;

    let stem = format!("taida_f52_no_check_handler_{}", std::process::id());
    let td_path = std::env::temp_dir().join(format!("{}.td", stem));
    let bin_path: PathBuf = std::env::temp_dir().join(stem);

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
    std::fs::write(&td_path, source).expect("write no-check F52 handler fixture");

    let output = Command::new(taida_bin())
        .args(["--no-check", "build", "native", "--handler", "handle"])
        .arg(&td_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .expect("run no-check native handler build");
    assert!(
        !output.status.success(),
        "--no-check handler build must still reject non-WebResponse body"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[E1601]") && stderr.contains("WebResponse"),
        "handler type surface should remain checked under --no-check: {}",
        stderr
    );

    let _ = std::fs::remove_file(&td_path);
    let _ = std::fs::remove_file(&bin_path);
}
