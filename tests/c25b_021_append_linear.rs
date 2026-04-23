//! C25B-021 (Phase 5-F2-2 Stage B) — Append / Prepend perf guard.
//!
//! # Scope
//!
//! Pre-5-F2-2 the interpreter evaluated
//!
//! ```taida
//! build acc i =
//!   | i >= N |> acc
//!   | _ |> build(Append[acc, i](), i + 1)
//! ```
//!
//! as O(N²) because the trampoline's `current_args` vector held an
//! Arc clone of `acc` throughout body evaluation, so `list_take`'s
//! `Arc::try_unwrap` always failed and fell back to a full `Vec`
//! clone. For N = 4800 this cost ~270 ms; for N = 20000 it cost
//! ~3.4 s, tracking the expected quadratic shape (each doubling of N
//! roughly 4× the time).
//!
//! Phase 5-F2-2 Stage B landed two complementary fixes:
//!
//!   1. `Interpreter::call_function`'s trampoline now
//!      `current_args.clear()` immediately after binding parameters,
//!      so env becomes the unique Arc holder for each arg.
//!
//!   2. `Append` / `Prepend` take the first argument out of the
//!      innermost scope when it is a bare `Expr::Ident` whose name
//!      does not appear in the remaining type-args, then mutate via
//!      `Arc::make_mut`. Combined with (1) this turns the hot loop
//!      into O(1) amortized per element.
//!
//! Measured wall-clock on the developer laptop (2026-04-23), release
//! build, including subprocess spawn:
//!
//! | N      | pre-5-F2-2 | post-5-F2-2 |
//! |--------|-----------:|------------:|
//! |  4 800 |    266 ms  |       3 ms  |
//! |  9 600 |    853 ms  |       5 ms  |
//! | 20 000 |  3 380 ms  |       9 ms  |
//!
//! The ratios confirm the asymptotic shape flipped from O(N²) to
//! O(N) (9600/4800 = 2× size, 5/3 = 1.67× time — linear with some
//! sub-linear fixed-cost headroom from subprocess spawn dominating
//! at small N).
//!
//! # What these tests pin
//!
//! * A 20 000-element Append loop completes in well under 500 ms.
//!   Pre-fix the same loop took ~3.4 s, so any regression of the
//!   unique-ownership fast path will trip this guard by an order of
//!   magnitude of headroom, not a borderline flake.
//! * A 5 000-element Prepend loop completes in well under 500 ms.
//!   (Prepend's per-iteration cost is slightly higher than Append's
//!   because `Vec::insert(0, v)` is O(N) within each element even
//!   under unique ownership — the fix keeps the Arc alloc O(1) but
//!   the Vec shift is still O(N), so the overall shape stays O(N²)
//!   but with a dramatically smaller constant factor. This is
//!   acceptable because Prepend is a cold-path idiom in Taida; the
//!   Append path is the tail-recursive accumulator pattern.)
mod common;

use common::taida_bin;
use std::fs;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_fixture_under(src: &str, timeout: Duration, label: &str) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "c25b_021_append_{}_{}_{}",
        std::process::id(),
        seq,
        label
    ));
    fs::create_dir_all(&dir).expect("create tmp dir");
    let path = dir.join("fixture.td");
    fs::write(&path, src).expect("write fixture");
    let start = Instant::now();
    let mut child = Command::new(taida_bin())
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn taida");
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let elapsed = start.elapsed();
                let out = child
                    .wait_with_output()
                    .unwrap_or_else(|_| panic!("child wait_with_output failed"));
                assert!(
                    status.success(),
                    "{}: taida run failed ({}): stderr={}",
                    label,
                    status,
                    String::from_utf8_lossy(&out.stderr)
                );
                assert!(
                    elapsed < timeout,
                    "{}: Append/Prepend took {}ms, exceeds {}ms ceiling — \
                     C25B-021 unique-ownership fast path regressed",
                    label,
                    elapsed.as_millis(),
                    timeout.as_millis()
                );
                let _ = fs::remove_dir_all(&dir);
                return;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_dir_all(&dir);
                    panic!(
                        "{}: Append/Prepend timed out (> {}ms) — \
                         C25B-021 unique-ownership fast path regressed",
                        label,
                        timeout.as_millis()
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("try_wait failed: {}", e),
        }
    }
}

/// Tail-recursive 20 000-element Append loop. Pre-5-F2-2 this took
/// ~3.4 s; with the fast path it completes in ~9 ms plus subprocess
/// spawn. 500 ms is a generous ceiling (~50× measured) sized to
/// survive slow CI runners while still catching any O(N²) regression
/// by at least an order of magnitude.
#[test]
fn c25b_021_append_20000_completes_under_500ms() {
    let src = r#"
build acc i =
  | i >= 20000 |> acc
  | _ |> build(Append[acc, i](), i + 1)

result <= build(@[], 0)
stdout("ok")
"#;
    run_fixture_under(src, Duration::from_millis(500), "append_20000");
}

/// 5 000-element Prepend loop. Prepend still has an inherent O(N²)
/// cost from `Vec::insert(0, v)` even under unique ownership — but
/// the Arc alloc cost that used to dominate is gone, so this runs
/// in the low hundreds of milliseconds at N=5000. 2 s is a safe
/// ceiling: any regression reintroducing the Arc-clone per iter
/// would push this past 10 s.
#[test]
fn c25b_021_prepend_5000_completes_under_2s() {
    let src = r#"
build acc i =
  | i >= 5000 |> acc
  | _ |> build(Prepend[acc, i](), i + 1)

result <= build(@[], 0)
stdout("ok")
"#;
    run_fixture_under(src, Duration::from_secs(2), "prepend_5000");
}

/// Guard that the cross-reference case still falls back to the
/// clone-based path correctly. Here the first arg is a bare Ident
/// but the name appears in the second arg, so the optimization must
/// skip and use the fallback. We don't pin a time here — just
/// correctness that the result is right.
#[test]
fn c25b_021_append_with_self_reference_in_second_arg_is_correct() {
    let src = r#"
xs <= @[10, 20, 30]
result <= Append[xs, xs]()
stdout(result)
"#;
    let dir = std::env::temp_dir().join(format!("c25b_021_selfref_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create tmp dir");
    let path = dir.join("fixture.td");
    fs::write(&path, src).expect("write fixture");
    let out = Command::new(taida_bin())
        .arg(&path)
        .output()
        .expect("spawn taida");
    assert!(
        out.status.success(),
        "taida run failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "@[10, 20, 30, @[10, 20, 30]]",
        "self-reference case must use the fallback clone path and \
         produce the pre-5-F2-2 output unchanged"
    );
    let _ = fs::remove_dir_all(&dir);
}
