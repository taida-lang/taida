#!/usr/bin/env python3
# C26B-004: Parse req/s + total=ms lines from RUN_BENCHMARKS=1 NET6 tests
# and emit a bencher-format line the `compare_baseline.py` gate consumes.
#
# The three integration tests covered here are listed in the C26B-004
# acceptance criteria:
#   - test_net6_3b_native_h2_32_request_throughput_benchmark
#   - test_net6_3b_native_h2_64kib_data_benchmark
#   - test_net6_3b_native_h2_32_stream_multiplex_benchmark
#
# Their eprintln!() output uses the form:
#   NET6-3b-<N> [...description...] ok=<A>/<B> req/s=<R> total=<T>ms | Design gate: ...
#
# We map each test to a synthetic bench name and emit nanoseconds per
# request (1e9 / req/s), so the `compare_baseline.py` gate can apply
# the same 10% regression tolerance.
#
# Usage:
#   parse_net_throughput.py --cargo-test-log cargo-test.log \
#       --output bench-results-net.txt
#
# Input is expected to contain the captured `cargo test -- --nocapture`
# output. Only matched lines are converted; un-matched benches are
# silently skipped so a missing h2 feature (curl --http2 not available)
# does not blow up the gate.

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# NET6-3b-2 [native h2 32 new-conn requests] ok=32/32 req/s=845.3 total=37ms | Design gate: ...
NET_RE = re.compile(
    r"^NET6-(?P<slug>[0-9a-z-]+)\s+\[(?P<desc>[^\]]+)\]\s+"
    r"ok=(?P<ok>\d+)/(?P<n>\d+)\s+"
    r"req/s=(?P<rps>[0-9.]+)\s+total=(?P<total>\d+)ms"
)

# slug -> bench name mapping. Names are stable (baseline JSON keys).
SLUG_TO_BENCH = {
    "3b-2": "net6_3b_native_h2_32_request_throughput",
    "3b-3": "net6_3b_native_h2_64kib_data",
    "3b-4": "net6_3b_native_h2_32_stream_multiplex",
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cargo-test-log", required=True)
    ap.add_argument("--output", required=True)
    args = ap.parse_args()

    log_path = Path(args.cargo_test_log)
    out_path = Path(args.output)

    if not log_path.exists():
        print(f"::warning::log not found: {log_path}", file=sys.stderr)
        out_path.write_text("")
        return 0

    lines_out: list[str] = []
    for raw in log_path.read_text(errors="replace").splitlines():
        m = NET_RE.search(raw.strip())
        if not m:
            continue
        slug = m.group("slug")
        bench = SLUG_TO_BENCH.get(slug)
        if bench is None:
            continue
        rps = float(m.group("rps"))
        if rps <= 0.0:
            continue
        # ns per request
        ns_per_req = 1_000_000_000.0 / rps
        # bencher format: `test <name> ... bench: <ns> ns/iter (+/- 0)`
        lines_out.append(
            f"test {bench} ... bench:     {int(ns_per_req):>12,} ns/iter (+/- 0)"
        )

    if not lines_out:
        print(
            "::warning::no NET6-3b-{2,3,4} benchmark lines matched. "
            "Benchmarks may have been skipped (curl --http2 unavailable).",
            file=sys.stderr,
        )

    out_path.write_text("\n".join(lines_out) + ("\n" if lines_out else ""))
    print(f"wrote {len(lines_out)} bencher line(s) -> {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
