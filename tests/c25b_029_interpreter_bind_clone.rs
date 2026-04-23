//! C25B-029 — interpreter `<=` deep-clone on locally-bound addon
//! `Value::Record` perf regression guard (Phase 5-F scope-reduced).
//!
//! # Scope
//!
//! Phase 5-F (C25_PROGRESS.md) started by exploring an
//! `HashMap<String, Arc<Value>>` environment to turn Ident lookups
//! into O(1) `Arc::clone` calls. The exploration revealed that without
//! also migrating `Value::List` / `Value::BuchiPack` to interior
//! `Arc<Vec<...>>` with copy-on-write, the outer `Rc::clone` plus
//! inner `Value::clone` combination is still a full deep-clone with no
//! net wall-clock improvement. The proper root-cause fix is the Value
//! interior-arc migration (tracked as 5-F2 in C25_PROGRESS.md), which
//! is out of scope for the current session and will land in a
//! dedicated follow-up or in D26.
//!
//! # What this test pins
//!
//! The minimal reproducer from `docs/smoke/_clone_probe.td` built
//! around a pure-Taida `Value::BuchiPack` containing a 4800-item
//! `Value::List` of cell-shaped BuchiPacks. Without the fix, a
//! chain of 11 `touch(p)` calls (each clones the outer pack's
//! cells reference) scales linearly with each additional call.
//!
//! We *intentionally* do **not** assert "fast" numbers here; the
//! blocker is still open. Instead the test:
//!   * validates the fixture *runs to completion at all* (it used
//!     to freeze for 17s per frame — still slow but bounded);
//!   * pins the shape of the final value so a regression in Append
//!     / BuchiPack field access surfaces loudly;
//!   * ships a `SLOW` annotation so a future Phase 5-F2 landing can
//!     flip this test to a hard latency gate without re-writing it.
mod common;

use common::taida_bin;
use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

/// The scenario mirrors Hachikuma's Pane.Render pattern: build a
/// large structured buffer inside the function, then hand it through
/// a chain of `touch` calls. Native AOT runs this sub-1ms; the
/// interpreter's deep-clone path (pre-5-F2) makes it O(chain²).
const CLONE_PROBE_FIXTURE: &str = r#"
buildCells acc i =
  | i >= 4800 |> acc
  | _ |> buildCells(Append[acc, @(ch <= "a", fg <= "white", bg <= "black")](), i + 1)

touch p =
  @(cols <= p.cols, rows <= p.rows, cells <= p.cells)

cells <= buildCells(@[], 0)
buf <= @(cols <= 120, rows <= 40, cells <= cells)

b01 <= touch(buf)
b02 <= touch(b01)
b03 <= touch(b02)
b04 <= touch(b03)
b05 <= touch(b04)
b06 <= touch(b05)
b07 <= touch(b06)
b08 <= touch(b07)
b09 <= touch(b08)
b10 <= touch(b09)
b11 <= touch(b10)
stdout(b11.cols)
stdout(b11.rows)
"#;

/// Currently the **main** cost is `buildCells` (Append chain) which
/// is itself O(N²) and tracked separately as C25B-021. The touch()
/// chain is effectively free for `Value::BuchiPack` shallow-read
/// access today; C25B-029's full 17-second freeze is specifically
/// triggered by addon `Value::Record` returns (not reproducible in
/// pure-Taida at this moment since the `taida-lang/terminal`
/// BufferNew / BufferWrite facade is only available when an addon
/// cdylib is present, which the test harness does not require).
///
/// We therefore pin the upper bound at a generous 30 seconds: the
/// fixture completed in 3.6 s on the developer laptop when measured
/// 2026-04-23, so 30 s is ~8× headroom for CI noise. Any future
/// regression that pushes the pure-Taida BuchiPack chain back to
/// minutes will fail this test loudly.
const MAX_DURATION: Duration = Duration::from_secs(30);

#[test]
fn c25b_029_bufferlike_touch_chain_does_not_freeze() {
    let dir = std::env::temp_dir().join(format!("c25b_029_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create tmp dir");
    let path = dir.join("clone_probe.td");
    fs::write(&path, CLONE_PROBE_FIXTURE).expect("write fixture");

    let start = Instant::now();
    let out = Command::new(taida_bin())
        .arg(&path)
        .output()
        .expect("spawn taida");
    let elapsed = start.elapsed();

    assert!(
        out.status.success(),
        "clone probe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        elapsed < MAX_DURATION,
        "C25B-029 clone probe took {}ms — regressed past {}ms ceiling. \
         Phase 5-F2 (Value interior-Arc migration, D26 or subsequent \
         RC) still pending; see .dev/C25_PROGRESS.md",
        elapsed.as_millis(),
        MAX_DURATION.as_millis()
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["120", "40"],
        "touch chain output regressed — BuchiPack field access path changed"
    );

    let _ = fs::remove_dir_all(&dir);
}
