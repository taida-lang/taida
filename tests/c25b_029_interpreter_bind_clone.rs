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
/// Phase 5-F2-1 (2026-04-23) migrated `Value::List` to interior
/// `Arc<Vec<Value>>`, collapsing the touch-chain cost (the 11
/// `touch(p)` calls) to O(chain) Arc refcount bumps instead of
/// O(chain × N) Vec deep-clones. That alone brought the fixture
/// from ~3.6 s to ~1.8 s but left the `Append` loop in
/// `buildCells` O(N²): the trampoline's `current_args` held an
/// Arc clone of `acc` across body evaluation, so `list_take`
/// always fell back to a full Vec clone.
///
/// Phase 5-F2-2 Stage B (2026-04-23, this commit) added two
/// complementary changes:
///
///   1. The trampoline now `current_args.clear()` immediately
///      after binding parameters, so the env becomes the unique
///      Arc holder for each arg.
///
///   2. `Append` / `Prepend` take the first argument out of the
///      innermost scope when it is a bare Ident whose name does
///      not appear in the remaining type-args, then use
///      `Arc::make_mut` to mutate in place. With (1) providing
///      the unique-ownership invariant, this turns the inner loop
///      into O(1) amortized per element (O(N) overall).
///
/// Measured wall-clock on the developer laptop (2026-04-23):
///
/// * pre-5-F2-1  (List=Vec)           ≈ 3.6 s
/// * post-5-F2-1 (List=Arc<Vec>)      ≈ 1.8 s
/// * post-5-F2-2 (this commit)        ≈ 0.004 s (four milliseconds)
///
/// The ceiling tightens from 5 s to **500 ms** — well above the
/// measured 4 ms but low enough that any accidental reintroduction
/// of the quadratic deep-clone shape will trip the guard within a
/// fraction of a CI budget rather than silently running seconds
/// long. Tightening further (e.g. to 50 ms) is risky because some
/// CI runners are 10× slower than the development machine on
/// subprocess spawn cost alone, and this test shells out to the
/// `taida` binary.
const MAX_DURATION: Duration = Duration::from_millis(500);

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
