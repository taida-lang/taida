//! Taida JS ランタイム — トランスパイル後の JS に埋め込むヘルパー関数。
//!
//! C12-9 (FB-21) で `src/js/runtime.rs` (6,496 行) を機能単位に分割した。
//! `RUNTIME_JS` は `LazyLock<&'static str>` で core / os / net の 3
//! チャンクを連結した const `&str` であり、従来の単一ファイル時代と
//! バイト単位で同一である。
//!
//! - [`core`]: helpers / 型 / 算術 / Lax / Result / BuchiPack / throw /
//!   Async / Regex / stream / stdout / stderr / stdin / format /
//!   toString / HashMap / Set / equals / typeof / spread
//!   (original lines 4..2003)
//! - [`os`]: `taida-lang/os` — readFile / writeFile / stat / listdir /
//!   exists / envvar / exec / http / process / pool + `sha256` crypto
//!   (original lines 2005..3137)
//! - [`net`]: `taida-lang/net` HTTP v1 runtime (parser / encoder /
//!   chunked / streaming writer / SSE / body reader / WebSocket)
//!   (original lines 3139..6381)
//!
//! 分割配置表: `.dev/taida-logs/docs/design/file_boundaries.md`

mod core;
mod net;
mod os;

use std::sync::LazyLock;

/// Full JS runtime source, assembled from the three chunks on first
/// access and cached for the process lifetime. Byte-identical to the
/// pre-split monolithic `RUNTIME_JS`.
///
/// C12-9 note: `concat!()` cannot be used because the macro requires
/// literal arguments; `LazyLock<&'static str>` + `Box::leak` exposes
/// a `&'static str` without adding a crate dependency.
pub static RUNTIME_JS: LazyLock<&'static str> = LazyLock::new(|| {
    let mut s = String::with_capacity(core::CORE_JS.len() + os::OS_JS.len() + net::NET_JS.len());
    s.push_str(core::CORE_JS);
    s.push_str(os::OS_JS);
    s.push_str(net::NET_JS);
    Box::leak(s.into_boxed_str())
});

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience: deref the `LazyLock<&'static str>` once per test.
    fn runtime() -> &'static str {
        *RUNTIME_JS
    }

    /// FL-28: Verify stdin does not use /dev/stdin (Windows incompatible)
    #[test]
    fn test_stdin_no_dev_stdin_hardcode() {
        let js = runtime();
        assert!(
            !js.contains("'/dev/stdin'"),
            "JS runtime should not hardcode '/dev/stdin' -- use process.stdin.fd for cross-platform compatibility"
        );
        assert!(
            !js.contains("\"/dev/stdin\""),
            "JS runtime should not hardcode \"/dev/stdin\""
        );
        // Verify the cross-platform approach is used
        assert!(
            js.contains("process.stdin.fd"),
            "JS runtime should use process.stdin.fd for cross-platform stdin"
        );
    }

    /// Regression: JS sseEvent must NOT build a single aggregate payload string.
    /// The old code concatenated everything into `ssePayload` before writing,
    /// defeating the zero-copy streaming design. The fix writes each SSE line
    /// separately within cork/uncork.
    #[test]
    fn test_sse_event_no_aggregate_payload_string() {
        let js = runtime();
        // The old pattern was: let ssePayload = ''; ssePayload += ...
        // followed by sock.write(ssePayload). After the fix, sseEvent should
        // write 'event: ' and 'data: ' lines directly to the socket.
        assert!(
            !js.contains("let ssePayload = ''"),
            "JS sseEvent should not build an aggregate ssePayload string — \
             write SSE lines directly to socket within cork/uncork"
        );
        assert!(
            !js.contains("sock.write(ssePayload)"),
            "JS sseEvent should not write a single aggregate ssePayload — \
             write each SSE line separately"
        );
    }

    /// Regression: JS writeChunk must track drain (backpressure) state.
    /// sock.write() returns false when the kernel buffer is full; the writer
    /// must record this so callers can react to backpressure.
    #[test]
    fn test_write_chunk_tracks_drain() {
        let js = runtime();
        assert!(
            js.contains("_needsDrain"),
            "JS writer should have a _needsDrain flag for backpressure tracking"
        );
        assert!(
            js.contains("'drain'"),
            "JS runtime should listen for 'drain' events to clear backpressure flag"
        );
    }

    /// Regression: JS drain listener must not accumulate across keep-alive
    /// requests.  The old code added `socket.on('drain', ...)` per request
    /// but never removed it, causing listener leak on keep-alive connections.
    /// The fix removes drain listeners in afterResponseWritten alongside
    /// timeout/end/error cleanup.
    #[test]
    fn test_drain_listener_cleaned_up_between_requests() {
        let js = runtime();
        // afterResponseWritten must remove drain listeners before re-attaching
        // for the next request (same pattern as timeout/end/error).
        assert!(
            js.contains("removeAllListeners('drain')"),
            "afterResponseWritten should remove drain listeners to prevent \
             accumulation on keep-alive connections"
        );
    }

    /// Regression: JS drain listener must store a removable reference.
    /// The old code used an anonymous function in `socket.on('drain', ...)`,
    /// making targeted removal impossible.  The fix stashes the handler
    /// as `writer._onDrain` for per-listener removal, and also uses
    /// `removeAllListeners('drain')` for bulk cleanup.
    #[test]
    fn test_drain_listener_has_named_reference() {
        let js = runtime();
        assert!(
            js.contains("_onDrain"),
            "drain listener should be stashed as _onDrain for removability"
        );
    }

    /// Contract: writeChunk/sseEvent are synchronous Unit functions.
    /// They must NOT return a Promise on backpressure — that would break
    /// the public contract (NET_DESIGN) and backend parity.
    /// Instead they only set `_needsDrain = true` for observability and
    /// always return undefined (Unit).
    #[test]
    fn test_backpressure_is_sync_unit() {
        let js = runtime();
        // writeChunk/sseEvent must set _needsDrain flag on backpressure
        assert!(
            js.contains("writer._needsDrain = true;"),
            "writeChunk/sseEvent should set _needsDrain flag on backpressure"
        );
        // Must NOT return a Promise (drain Promise would break sync Unit contract)
        assert!(
            !js.contains("return new Promise(function(resolve) { writer._drainResolve"),
            "writeChunk/sseEvent must NOT return a drain Promise — \
             they are synchronous Unit per NET_DESIGN contract"
        );
        // Must NOT have _drainResolve field (no async drain resolution)
        assert!(
            !js.contains("_drainResolve"),
            "writer must NOT have _drainResolve field — \
             backpressure is handled by Node.js internal buffering"
        );
    }

    /// C12-9 (FB-21): guard the split invariants. Verifies chunk
    /// boundaries are preserved exactly once in the concat output.
    #[test]
    fn test_runtime_js_chunk_concat_invariants() {
        let js = runtime();
        // Each chunk contributes a unique anchor that must appear exactly once:
        assert_eq!(
            js.matches("function __taida_trampoline(").count(),
            1,
            "__taida_trampoline must be defined exactly once (core chunk)"
        );
        assert_eq!(
            js.matches("// ── taida-lang/os — Core-bundled OS package")
                .count(),
            1,
            "OS header must appear exactly once (os chunk boundary)"
        );
        assert_eq!(
            js.matches("function __taida_net_result_ok").count(),
            1,
            "net result_ok helper must be defined exactly once (net chunk)"
        );
        // Byte-length sanity: concat must not lose or add bytes.
        assert_eq!(
            js.len(),
            core::CORE_JS.len() + os::OS_JS.len() + net::NET_JS.len(),
        );
    }
}
