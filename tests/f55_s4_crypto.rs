//! F55 S4 — `taida-lang/crypto` surface expansion.
//!
//! The crypto package gains SHA-512 / 384 / 224, HMAC-SHA256,
//! constant-time equality, hex / base64 encode + decode, and randomBytes.
//! sha256's contract (lower-hex `Str`, `Str|Bytes` input, `[E1506]`) is
//! unchanged.
//!
//! These tests pin:
//!   (a) each hash against its NIST known-answer vector with 4-backend
//!       parity (interpreter / native / JS) plus a wasm-wasi / wasm-full
//!       parity check,
//!   (b) HMAC-SHA256 against RFC 4231 test cases,
//!   (c) hex / base64 round-trips and the failure side of `Lax[Bytes]` for
//!       malformed input,
//!   (d) constantTimeEquals true / false,
//!   (e) randomBytes: requested length, non-determinism, the empty case,
//!       and the deterministic wasm-min / wasm-edge compile-time reject,
//!   (f) sha256 regression (unchanged surface).
//!
//! All hash / hmac / encode symbols are pure `Str` / `Bool`, available on
//! every backend including wasm-min. The Bytes-producing decode / random
//! symbols are wasm-wasi / wasm-full only (compile-time reject on
//! wasm-min / wasm-edge), so their wasm parity is checked on wasi / full.

mod common;

use common::{taida_bin, wasmtime_bin};
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique(label: &str, ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "f55s4_{}_{}_{}{}",
        label,
        std::process::id(),
        nanos,
        ext
    ))
}

fn write_td(label: &str, source: &str) -> PathBuf {
    let td = unique(label, ".td");
    std::fs::write(&td, source).expect("write .td");
    td
}

fn run_interp(td: &Path) -> Option<String> {
    let out = Command::new(taida_bin()).arg(td).output().ok()?;
    if !out.status.success() {
        eprintln!("interp failed: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn run_native(td: &Path, label: &str) -> Option<String> {
    let exe = unique(&format!("{}_n", label), "");
    let build = Command::new(taida_bin())
        .args(["build", "native"])
        .arg(td)
        .arg("-o")
        .arg(&exe)
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "native build failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let out = Command::new(&exe).output().ok()?;
    let _ = std::fs::remove_file(&exe);
    if !out.status.success() {
        eprintln!(
            "native run failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

fn run_js(td: &Path, label: &str) -> Option<String> {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("SKIP: node unavailable");
        return None;
    }
    let js = unique(&format!("{}_js", label), ".mjs");
    let build = Command::new(taida_bin())
        .args(["build", "js"])
        .arg(td)
        .arg("-o")
        .arg(&js)
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "js build failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let out = Command::new("node").arg(&js).output().ok()?;
    let _ = std::fs::remove_file(&js);
    if !out.status.success() {
        eprintln!("js run failed: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Compile + run on a wasm profile via wasmtime. `None` when wasmtime is
/// unavailable (skip) or compilation fails.
fn run_wasm(td: &Path, profile: &str, label: &str) -> Option<String> {
    let wasmtime = wasmtime_bin()?;
    let wasm = unique(&format!("{}_{}", label, profile), ".wasm");
    let build = Command::new(taida_bin())
        .args(["build", profile])
        .arg(td)
        .arg("-o")
        .arg(&wasm)
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "{} build failed: {}",
            profile,
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let out = Command::new(&wasmtime)
        .args(["run", "--"])
        .arg(&wasm)
        .output()
        .ok()?;
    let _ = std::fs::remove_file(&wasm);
    if !out.status.success() {
        eprintln!(
            "{} run failed: {}",
            profile,
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Assert that a fixture produces `expected` on interpreter + native + JS,
/// and (when wasmtime is present) on wasm-wasi + wasm-full.
fn assert_parity(label: &str, source: &str, expected: &str) {
    let td = write_td(label, source);

    let interp = run_interp(&td).expect("interpreter run");
    assert_eq!(interp, expected, "[{}] interpreter mismatch", label);

    let native = run_native(&td, label).expect("native run");
    assert_eq!(native, expected, "[{}] native parity", label);

    if let Some(js) = run_js(&td, label) {
        assert_eq!(js, expected, "[{}] JS parity", label);
    }

    if wasmtime_bin().is_some() {
        let wasi = run_wasm(&td, "wasm-wasi", label).expect("wasm-wasi run");
        assert_eq!(wasi, expected, "[{}] wasm-wasi parity", label);
        let full = run_wasm(&td, "wasm-full", label).expect("wasm-full run");
        assert_eq!(full, expected, "[{}] wasm-full parity", label);
    } else {
        eprintln!("SKIP: wasmtime unavailable — wasm parity not verified");
    }

    let _ = std::fs::remove_file(&td);
}

/// Parity for a fixture whose surface is pure Str/Bool (works on every wasm
/// profile, including wasm-min / wasm-edge).
fn assert_parity_all_wasm(label: &str, source: &str, expected: &str) {
    assert_parity(label, source, expected);
    if wasmtime_bin().is_some() {
        let td = write_td(&format!("{}_minedge", label), source);
        let min = run_wasm(&td, "wasm-min", label).expect("wasm-min run");
        assert_eq!(min, expected, "[{}] wasm-min parity", label);
        let edge = run_wasm(&td, "wasm-edge", label).expect("wasm-edge run");
        assert_eq!(edge, expected, "[{}] wasm-edge parity", label);
        let _ = std::fs::remove_file(&td);
    }
}

// ── (a) hash known vectors ──────────────────────────────────────────

#[test]
fn sha256_abc_vector_parity() {
    assert_parity_all_wasm(
        "sha256_abc",
        ">>> taida-lang/crypto => @(sha256)\nstdout(sha256(\"abc\"))\n",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
}

#[test]
fn sha512_vectors_parity() {
    // NIST: SHA-512("") and SHA-512("abc").
    assert_parity_all_wasm(
        "sha512",
        ">>> taida-lang/crypto => @(sha512)\nstdout(sha512(\"\"))\nstdout(sha512(\"abc\"))\n",
        "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e\n\
         ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
    );
}

#[test]
fn sha384_abc_vector_parity() {
    assert_parity_all_wasm(
        "sha384",
        ">>> taida-lang/crypto => @(sha384)\nstdout(sha384(\"abc\"))\n",
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
    );
}

#[test]
fn sha224_abc_vector_parity() {
    assert_parity_all_wasm(
        "sha224",
        ">>> taida-lang/crypto => @(sha224)\nstdout(sha224(\"abc\"))\n",
        "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
    );
}

// ── (b) HMAC-SHA256 RFC 4231 ────────────────────────────────────────

#[test]
fn hmac_sha256_rfc4231_parity() {
    // Case 2: key="Jefe", data="what do ya want for nothing?".
    assert_parity_all_wasm(
        "hmac",
        ">>> taida-lang/crypto => @(hmacSha256)\nstdout(hmacSha256(\"Jefe\", \"what do ya want for nothing?\"))\n",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    );
}

// ── (c) hex / base64 round-trip + invalid decode Lax failure ────────

#[test]
fn hex_roundtrip_and_invalid_parity() {
    // hexEncode("Hi") -> "4869"; hexDecode round-trips; invalid hex -> Lax
    // empty (has_value = false).
    assert_parity(
        "hex",
        r#">>> taida-lang/crypto => @(hexEncode, hexDecode)
stdout(hexEncode("Hi"))
hexDecode("4869") >=> d
stdout(hexEncode(d))
stdout(hexDecode("zz").has_value.toString())
stdout(hexDecode("4869").has_value.toString())
"#,
        "4869\n4869\nfalse\ntrue",
    );
}

#[test]
fn base64_roundtrip_and_invalid_parity() {
    assert_parity(
        "b64",
        r#">>> taida-lang/crypto => @(base64Encode, base64Decode, hexEncode)
stdout(base64Encode("foobar"))
base64Decode("Zm9vYmFy") >=> b
stdout(hexEncode(b))
stdout(base64Decode("Zm9").has_value.toString())
stdout(base64Decode("Zm9v").has_value.toString())
"#,
        "Zm9vYmFy\n666f6f626172\nfalse\ntrue",
    );
}

// ── (d) constantTimeEquals true / false ─────────────────────────────

#[test]
fn constant_time_equals_parity() {
    assert_parity_all_wasm(
        "cteq",
        r#">>> taida-lang/crypto => @(constantTimeEquals)
stdout(constantTimeEquals("secret", "secret").toString())
stdout(constantTimeEquals("secret", "secreT").toString())
stdout(constantTimeEquals("ab", "abc").toString())
"#,
        "true\nfalse\nfalse",
    );
}

// ── (e) randomBytes: length + non-determinism + empty + wasm gate ───

#[test]
fn random_bytes_length_and_nondeterminism_parity() {
    // randomBytes returns a plain Bytes (not a Lax/Mold), so it binds with
    // `<=`. Two 16-byte draws colliding is cryptographically impossible, so
    // the smoke output is deterministic: length 16 + "different".
    assert_parity(
        "rand",
        r#">>> taida-lang/crypto => @(randomBytes, constantTimeEquals)
a <= randomBytes(16)
b <= randomBytes(16)
stdout(a.length().toString())
stdout(constantTimeEquals(a, b).toString())
e <= randomBytes(0)
stdout(e.length().toString())
"#,
        "16\nfalse\n0",
    );
}

fn build_wasm_expect_reject(td: &Path, profile: &str) -> String {
    let wasm = unique(&format!("reject_{}", profile), ".wasm");
    let out = Command::new(taida_bin())
        .args(["build", profile])
        .arg(td)
        .arg("-o")
        .arg(&wasm)
        .output()
        .expect("spawn taida build");
    let _ = std::fs::remove_file(&wasm);
    assert!(
        !out.status.success(),
        "{} must reject randomBytes at compile time",
        profile
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn random_bytes_rejected_on_wasm_min_and_edge() {
    let td = write_td(
        "rand_reject",
        ">>> taida-lang/crypto => @(randomBytes)\na <= randomBytes(8)\nstdout(a.length().toString())\n",
    );
    for profile in ["wasm-min", "wasm-edge"] {
        let stderr = build_wasm_expect_reject(&td, profile);
        assert!(
            stderr.contains("taida_crypto_random_bytes") && stderr.contains("does not support"),
            "{} reject message must name the unsupported runtime function; got: {}",
            profile,
            stderr
        );
    }
    let _ = std::fs::remove_file(&td);
}

#[test]
fn hex_decode_rejected_on_wasm_min() {
    // Bytes-producing decoders are also gated to wasm-wasi / wasm-full.
    let td = write_td(
        "hexdec_reject",
        ">>> taida-lang/crypto => @(hexDecode)\nhexDecode(\"4869\") >=> d\nstdout(d.length().toString())\n",
    );
    let stderr = build_wasm_expect_reject(&td, "wasm-min");
    assert!(
        stderr.contains("taida_crypto_hex_decode") && stderr.contains("does not support"),
        "wasm-min reject message must name taida_crypto_hex_decode; got: {}",
        stderr
    );
    let _ = std::fs::remove_file(&td);
}

// ── (f) checker: per-symbol [E1506] argument validation ─────────────

#[test]
fn argument_type_errors_use_e1506() {
    for (label, source) in [
        (
            "hash_int",
            ">>> taida-lang/crypto => @(sha512)\nstdout(sha512(42))\n",
        ),
        (
            "random_str",
            ">>> taida-lang/crypto => @(randomBytes)\na <= randomBytes(\"x\")\nstdout(a.length().toString())\n",
        ),
        (
            "decode_int",
            ">>> taida-lang/crypto => @(hexDecode)\nhexDecode(99) >=> d\nstdout(d.length().toString())\n",
        ),
    ] {
        let td = write_td(label, source);
        let out = Command::new(taida_bin())
            .arg(&td)
            .output()
            .expect("spawn taida");
        let _ = std::fs::remove_file(&td);
        assert!(!out.status.success(), "[{}] type error must reject", label);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("[E1506]"),
            "[{}] expected [E1506], got: {}",
            label,
            stderr
        );
    }
}

/// Checker-level arity shortfall: crypto functions have a fixed ABI on
/// every backend, so a hole-free call with missing arguments must be
/// rejected at compile time (`[E1301]`) instead of reaching lowering.
/// A pipeline stage call only counts the injected pipe value as one
/// argument — a stage that is still short after the injection is
/// rejected too.
#[test]
fn arity_shortfall_errors_use_e1301() {
    for (label, source) in [
        (
            "hmac_one_arg",
            ">>> taida-lang/crypto => @(hmacSha256)\nstdout(hmacSha256(\"key\"))\n",
        ),
        (
            "cteq_one_arg",
            ">>> taida-lang/crypto => @(constantTimeEquals)\nstdout(constantTimeEquals(\"a\").toString())\n",
        ),
        (
            "random_zero_arg",
            ">>> taida-lang/crypto => @(randomBytes)\na <= randomBytes()\nstdout(a.length().toString())\n",
        ),
        (
            "sha256_zero_arg",
            ">>> taida-lang/crypto => @(sha256)\nstdout(sha256())\n",
        ),
        (
            "hmac_pipeline_still_short",
            ">>> taida-lang/crypto => @(hmacSha256)\n\"k\" => hmacSha256() => stdout(_)\n",
        ),
        (
            "hex_encode_nested_in_stage_args",
            // The implicit injection belongs to the stage call only — a
            // zero-argument call nested in the stage's arguments is still
            // an arity shortfall.
            ">>> taida-lang/crypto => @(sha256, hexEncode)\n\"abc\" => sha256(hexEncode()) => stdout(_)\n",
        ),
    ] {
        let td = write_td(label, source);
        let out = Command::new(taida_bin())
            .arg(&td)
            .output()
            .expect("spawn taida");
        let _ = std::fs::remove_file(&td);
        assert!(
            !out.status.success(),
            "[{}] arity shortfall must reject",
            label
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("[E1301]"),
            "[{}] expected [E1301], got: {}",
            label,
            stderr
        );
    }
}

/// The injection also counts toward the *excess* side: a pipeline stage
/// call whose written arguments already fill the arity overflows the
/// fixed ABI once the piped value is prepended (`"abc" => sha256("x")`
/// lowers to a 2-value call of a 1-ary builtin — the interpreter guards
/// at runtime, native fails at build time, and JS silently misruns).
/// Such calls must be rejected at check time.
#[test]
fn pipeline_excess_args_use_e1301() {
    for (label, source) in [
        (
            "sha256_pipeline_excess",
            ">>> taida-lang/crypto => @(sha256)\n\"abc\" => sha256(\"x\") => stdout(_)\n",
        ),
        (
            "hmac_pipeline_excess",
            ">>> taida-lang/crypto => @(hmacSha256)\n\"data\" => hmacSha256(\"key\", \"extra\") => stdout(_)\n",
        ),
        (
            "random_pipeline_excess",
            ">>> taida-lang/crypto => @(randomBytes)\n32 => randomBytes(1) => stdout(_.length().toString())\n",
        ),
    ] {
        let td = write_td(label, source);
        let out = Command::new(taida_bin())
            .arg(&td)
            .output()
            .expect("spawn taida");
        let _ = std::fs::remove_file(&td);
        assert!(
            !out.status.success(),
            "[{}] pipeline excess args must reject",
            label
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("[E1301]"),
            "[{}] expected [E1301], got: {}",
            label,
            stderr
        );
    }
}

/// The injected pipe value is the call's first argument, so it must also
/// satisfy the per-symbol argument-type rule. A mistyped piped value used
/// to pass the checker (the interpreter then failed at runtime while
/// native built and silently misran — e.g. a Bool piped into sha256
/// hashed the empty string). Such calls must be rejected with `[E1506]`
/// at check time.
#[test]
fn pipeline_injected_arg_type_errors_use_e1506() {
    for (label, source) in [
        (
            "bool_into_sha256",
            ">>> taida-lang/crypto => @(sha256)\ntrue => sha256() => stdout(_)\n",
        ),
        (
            "str_into_random_bytes",
            ">>> taida-lang/crypto => @(randomBytes)\n\"abc\" => randomBytes() => stdout(_.length().toString())\n",
        ),
        (
            "int_into_hmac",
            ">>> taida-lang/crypto => @(hmacSha256)\n1 => hmacSha256(\"key\") => stdout(_)\n",
        ),
    ] {
        let td = write_td(label, source);
        let out = Command::new(taida_bin())
            .arg(&td)
            .output()
            .expect("spawn taida");
        let _ = std::fs::remove_file(&td);
        assert!(
            !out.status.success(),
            "[{}] mistyped piped value must reject",
            label
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("[E1506]"),
            "[{}] expected [E1506], got: {}",
            label,
            stderr
        );
    }
}

/// A placeholder-free pipeline stage call receives the piped value as an
/// implicit first argument at runtime (`"abc" => sha256()` runs as
/// `sha256("abc")`). The fixed-arity validation must count that injection
/// instead of rejecting the stage call, the substitution form
/// (`sha256(_)`) must keep working, and the injected call must agree with
/// the direct call on every backend. (The injected forms here return
/// `Str` — plus a length-only randomBytes probe — so the pinned output is
/// display-stable across backends.)
#[test]
fn pipeline_first_arg_injection_parity() {
    assert_parity(
        "pipe_inject",
        r#">>> taida-lang/crypto => @(sha256, hmacSha256, randomBytes)
"abc" => sha256() => stdout(_)
"abc" => sha256(_) => stdout(_)
"data" => hmacSha256("key") => stdout(_)
stdout(hmacSha256("data", "key"))
32 => randomBytes() => stdout(_.length().toString())
"#,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
         ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
         fc4696cc790452088a25939f0befe2f1d960b7f8ed37266d5c84a6d8cbbbba31\n\
         fc4696cc790452088a25939f0befe2f1d960b7f8ed37266d5c84a6d8cbbbba31\n\
         32",
    );
}
