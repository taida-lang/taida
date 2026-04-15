//! Taida native runtime — single translation unit assembled from seven
//! mechanical fragments.
//!
//! C12B-026 (C12-9 Phase 9 Step 2) で `src/codegen/native_runtime.c`
//! (20,866 行 / 886,457 bytes) を機能単位で 7 フラグメントに分割した。
//!
//! **採用方式**: Rust-level 連結 (`LazyLock<&'static str>` + `Box::leak`) —
//! `runtime_core_wasm/` (C12-7a / C12B-027) と同一。clang 視点では完全に
//! 1 translation unit として振る舞う (driver.rs が `fs::write` で連結済み
//! の C ソースを書き出してから clang に渡す) ため、DCE / static helper
//! の cross-reference / forward declaration は分割前とバイト単位で同一。
//!
//! - [`CORE_SECTION`] (5,116 行): libc stubs / safe-malloc / allocator /
//!   type conversion molds / ref-counting / heap strings / BuchiPack /
//!   globals / Closure / List / Bytes / String / Regex (C12-6) /
//!   polymorphic dispatchers / template strings / Int/Float/Bool/Num
//!   methods / HashMap / Set / polymorphic length / collection methods
//!   (元 lines 1..5116)
//! - [`ERROR_JSON_SECTION`] (2,722 行): Error ceiling (setjmp/longjmp) /
//!   Result / Lax methods / polymorphic monadic dispatch / Async pthread
//!   support / Async aggregation / Debug for list / JSON Molten Iron /
//!   stdlib math / Field registry / jsonEncode/jsonPretty / stdlib I/O /
//!   SHA-256
//!   (元 lines 5117..7838)
//! - [`OS_SECTION`] (668 行): taida-lang/os package (Read / readBytes /
//!   ListDir / Stat / Exists / EnvVar / writeFile / writeBytes /
//!   appendFile / remove / createDir / rename / run / execShell /
//!   allEnv / ReadAsync)
//!   (元 lines 7839..8506)
//! - [`TLS_TCP_SECTION`] (1,720 行): NET5-4a OpenSSL dlopen / TLS-aware I/O
//!   wrappers / HTTP/1.1 over raw TCP / TCP socket APIs / pool package
//!   runtime
//!   (元 lines 8507..10226)
//! - [`NET_V1_SECTION`] (4,117 行): taida-lang/net HTTP v1 runtime
//!   (httpParseRequestHead / httpEncodeResponse / readBody / keep-alive /
//!   chunked compaction / httpServe helpers / v3 streaming writer /
//!   v4 body streaming / WebSocket / wsUpgrade / wsSend / wsReceive /
//!   wsClose / thread pool / worker thread)
//!   (元 lines 10227..14343)
//! - [`NET_H2_SECTION`] (2,065 行): Native HTTP/2 server (NET6-3a h2
//!   parity / HPACK static & dynamic tables / HPACK Huffman / HPACK
//!   int/string coding / H2 stream state / H2 frame I/O / H2 response
//!   send / H2 frame processing / H2 request & response extraction /
//!   serve one connection / taida_net_h2_serve)
//!   (元 lines 14344..16408)
//! - [`NET_H3_MAIN_SECTION`] (4,458 行): H3/QPACK constants / QPACK
//!   static & dynamic tables / QPACK int/string/header coding /
//!   H3 stream state / H3 varint / H3 frame I/O / SETTINGS / GOAWAY /
//!   H3 request/response path / NET7-8a libquiche dlopen FFI /
//!   QPACK encoder/decoder instruction streams / H3 self-tests /
//!   NET7-8b QUIC connection pool / serve_h3_loop / taida_net_h3_serve /
//!   httpServe entry / RC2.5 addon dispatch / main()
//!   (元 lines 16409..20866)
//!
//! 分割配置表: `.dev/taida-logs/docs/design/file_boundaries.md §4`

use std::sync::LazyLock;

/// Fragment 1: core primitives (5,116 lines).
pub const CORE_SECTION: &str = include_str!("01_core.inc.c");

/// Fragment 2: error ceiling, Result/Lax/Async, JSON, SHA-256 (2,722 lines).
pub const ERROR_JSON_SECTION: &str = include_str!("02_error_json.inc.c");

/// Fragment 3: taida-lang/os package (668 lines).
pub const OS_SECTION: &str = include_str!("03_os.inc.c");

/// Fragment 4: OpenSSL TLS, TCP sockets, pool (1,720 lines).
pub const TLS_TCP_SECTION: &str = include_str!("04_tls_tcp.inc.c");

/// Fragment 5: HTTP v1 runtime, streaming, WebSocket, thread pool (4,117 lines).
pub const NET_V1_SECTION: &str = include_str!("05_net_v1.inc.c");

/// Fragment 6: Native HTTP/2 server (2,065 lines).
pub const NET_H2_SECTION: &str = include_str!("06_net_h2.inc.c");

/// Fragment 7: HTTP/3 + QPACK + QUIC + httpServe + addon dispatch + main()
/// (4,458 lines).
pub const NET_H3_MAIN_SECTION: &str = include_str!("07_net_h3_main.inc.c");

/// Full native runtime C source, assembled from the seven fragments on
/// first access and cached for the process lifetime.
///
/// Byte-identical to the pre-split monolithic `native_runtime.c` — see
/// `test_native_runtime_fragment_concat_preserves_bytes` below for the
/// invariant assertion.
///
/// C12B-026 note: `concat!()` cannot be used because that macro requires
/// literal arguments; `LazyLock<&'static str>` + `Box::leak` exposes a
/// `&'static str` without adding a crate dependency. Same strategy as
/// `src/codegen/runtime_core_wasm/mod.rs::RUNTIME_CORE_WASM` (C12-7a /
/// C12B-027) and `src/js/runtime/mod.rs::RUNTIME_JS` (C12-9d).
pub static NATIVE_RUNTIME_C: LazyLock<&'static str> = LazyLock::new(|| {
    let total = CORE_SECTION.len()
        + ERROR_JSON_SECTION.len()
        + OS_SECTION.len()
        + TLS_TCP_SECTION.len()
        + NET_V1_SECTION.len()
        + NET_H2_SECTION.len()
        + NET_H3_MAIN_SECTION.len();
    let mut s = String::with_capacity(total);
    s.push_str(CORE_SECTION);
    s.push_str(ERROR_JSON_SECTION);
    s.push_str(OS_SECTION);
    s.push_str(TLS_TCP_SECTION);
    s.push_str(NET_V1_SECTION);
    s.push_str(NET_H2_SECTION);
    s.push_str(NET_H3_MAIN_SECTION);
    debug_assert_eq!(s.len(), total);
    Box::leak(s.into_boxed_str())
});

#[cfg(test)]
mod tests {
    use super::*;

    /// C12B-026 invariant: the concatenation of the seven fragments must be
    /// byte-identical to the pre-split monolithic `native_runtime.c`.
    /// We anchor the total byte length + a check of the first / last
    /// meaningful lines of the assembled source to detect accidental edits
    /// that would break DCE or shift static helper references across
    /// fragment boundaries.
    ///
    /// Total bytes snapshot: 886,457 (at C12B-026 split time). If a future
    /// change intentionally modifies the runtime C source, update both the
    /// relevant fragment file and the `EXPECTED_TOTAL_LEN` constant below
    /// in the same commit.
    #[test]
    fn test_native_runtime_fragment_concat_preserves_bytes() {
        const EXPECTED_TOTAL_LEN: usize = 886_457;
        let asm = *NATIVE_RUNTIME_C;
        assert_eq!(
            asm.len(),
            EXPECTED_TOTAL_LEN,
            "native_runtime fragments concatenate to unexpected size. \
             If you modified the C source deliberately, update EXPECTED_TOTAL_LEN."
        );
        // Anchor the first bytes of the assembled source to the historical
        // stdio.h include so accidental reordering of fragments is caught.
        assert!(
            asm.starts_with("#include <stdio.h>\n"),
            "first bytes of assembled source must start with <stdio.h> include"
        );
        // Anchor the tail of the assembled source to the closing brace of
        // main() — catches accidental truncation of the tail fragment.
        assert!(
            asm.trim_end().ends_with("(void)_taida_main();\n    return 0;\n}"),
            "tail of assembled source must end with main() body + closing brace"
        );
    }

    /// Each fragment must be a proper C suffix / prefix — no fragment
    /// should begin mid-statement. Fragment 1 starts with the `#include`
    /// preamble; fragments 2-7 each begin with a `// ──` section divider
    /// comment at column 0.
    #[test]
    fn test_native_runtime_fragment_boundaries_are_top_level() {
        assert!(
            CORE_SECTION.starts_with("#include <stdio.h>"),
            "fragment 1 (core) must begin with the <stdio.h> include"
        );
        for (name, frag) in [
            ("error_json", ERROR_JSON_SECTION),
            ("os", OS_SECTION),
            ("tls_tcp", TLS_TCP_SECTION),
            ("net_v1", NET_V1_SECTION),
            ("net_h2", NET_H2_SECTION),
            ("net_h3_main", NET_H3_MAIN_SECTION),
        ] {
            let first = frag.lines().next().unwrap_or("");
            assert!(
                first.starts_with("// ──") || first.starts_with("/*") || first.is_empty(),
                "fragment '{}' must begin at a top-level boundary (found: {:?})",
                name,
                first
            );
        }
    }

    /// Smoke test that none of the fragments are empty or suspiciously
    /// small (would indicate a boundary mis-calculation).
    #[test]
    fn test_native_runtime_fragments_nonempty() {
        assert!(CORE_SECTION.len() > 100_000, "core fragment suspiciously small");
        assert!(
            ERROR_JSON_SECTION.len() > 50_000,
            "error_json fragment suspiciously small"
        );
        assert!(OS_SECTION.len() > 10_000, "os fragment suspiciously small");
        assert!(
            TLS_TCP_SECTION.len() > 30_000,
            "tls_tcp fragment suspiciously small"
        );
        assert!(
            NET_V1_SECTION.len() > 100_000,
            "net_v1 fragment suspiciously small"
        );
        assert!(
            NET_H2_SECTION.len() > 50_000,
            "net_h2 fragment suspiciously small"
        );
        assert!(
            NET_H3_MAIN_SECTION.len() > 100_000,
            "net_h3_main fragment suspiciously small"
        );
    }
}
