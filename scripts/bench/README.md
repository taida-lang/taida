# scripts/bench/

Perf regression gate helpers for `@c.26`. Owned by `.github/workflows/bench.yml`
(C26B-004).

## Files

| File | Purpose |
|------|---------|
| `compare_baseline.py` | Hard-fail regression gate. Compares current bencher output against `.github/bench-baselines/perf_baseline.json` with a +10% tolerance. |
| `update_baseline.py` | Main-branch collector. Advances the baseline JSON via EWMA median and increments `sample_count`. |
| `parse_net_throughput.py` | Extracts `req/s` from the RUN_BENCHMARKS=1 NET6-3b h2 throughput tests and emits bencher-format lines for `compare_baseline.py`. |

## C26B-004 state machine

- **PR runs** call `compare_baseline.py`. Benches with
  `sample_count < min_samples_required` (default 30) emit WARN and
  do not fail. Benches that cleared the threshold fail the job
  when they regress beyond the tolerance.
- **main push runs** also call `update_baseline.py` and commit the
  refreshed `perf_baseline.json` back to `main`.
- **Scheduled / dispatched runs** behave like a PR run (gate-only).

## Baseline schema

```json
{
  "schema_version": 1,
  "min_samples_required": 30,
  "tolerance_pct": 10.0,
  "benches": {
    "<bench_name>": {
      "ns_median": <float>,
      "sample_count": <int>,
      "notes": "..."
    }
  }
}
```

`ns_median` is maintained as an EWMA with `alpha = 1/min(sample_count+1, 10)`
so the baseline absorbs samples slowly once it is warm and does not
over-fit a single transient spike.

## Why three h2 NET benches are derived via `parse_net_throughput.py`

The acceptance criteria for C26B-004 list three NET6-3b h2 throughput
tests (`test_net6_3b_native_h2_32_request_throughput_benchmark` /
`test_net6_3b_native_h2_64kib_data_benchmark` /
`test_net6_3b_native_h2_32_stream_multiplex_benchmark`). Those live in
`tests/parity.rs` as RUN_BENCHMARKS=1-gated integration tests (they
need real curl + openssl + a live native server). Instead of porting
them to criterion, `bench.yml` runs them as integration tests and
`parse_net_throughput.py` maps their `req/s` metric to a synthetic
bencher line (`ns = 1e9 / req_per_s`) so the same
`compare_baseline.py` gate applies uniformly.

If curl / openssl / h2 feature are unavailable on the runner, the
integration tests auto-SKIP and `parse_net_throughput.py` produces an
empty bencher file — the gate does not fail for absent benches
(it fails only on regressions of benches that were observed _and_
have a warm baseline).
