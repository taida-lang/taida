#!/usr/bin/env bash
# C26B-005 / C27B-015 / C27B-017 fast-soak proxy.
#
# This is a **proxy** for the 24h NET scatter-gather soak test pinned
# at `.dev/C26_SOAK_RUNBOOK.md`. The real acceptance is the full 24h
# run; this 30-minute helper exists so an operator can get a
# first-order signal on leak / drift regressions during development
# without committing a full day to each iteration. A PASS from this
# proxy does **not** close the C26B-005 acceptance — only a documented
# 24h PASS recorded in `.dev/C26_SOAK_RUNBOOK.md` does.
#
# Usage:
#   ./scripts/soak/fast-soak-proxy.sh [--duration-min N] [--backend interp|js|native]
#
# Backend dispatch (C27B-015):
#   --backend interp  → target/release/taida <fixture>           (port 18081)
#   --backend js      → taida build --target js --run + node     (port 18082)
#   --backend native  → taida build --target native --run        (port 18083)
#
# Each backend uses a distinct port so all three can be launched in
# parallel from three terminals (or three CI shards) without bind
# contention. CSV / log / projection paths are per-invocation
# tempdirs, so parallel runs do not clobber each other.
#
# A backend's exit code is independent — JS / native build failures
# cannot be hidden by a successful interpreter run, because each
# invocation starts at most one server process.
#
# Optional env vars:
#   TAIDA_NET_ANNOUNCE_PORT=1  Forwarded to the spawned server. Causes
#                              `httpServe` to print one stdout line of
#                              the form `listening on 127.0.0.1:NNNNN`
#                              before the first accept(). Default OFF
#                              (production stdout surface unchanged).
#                              Enable when you want to read the bound
#                              port back from the script's `LOG`.
#                              See `.dev/C27_BLOCKERS.md::C27B-014`.
#
# CI smoke (C27B-017): `.github/workflows/soak-smoke.yml` runs this
# script with `--duration-min 1 --backend interp`. The job fails
# fast on parse errors, bind failures, and missing VERDICT lines.
#
# The script:
#   1. Builds a release binary.
#   2. Launches `examples/quality/c26_soak_fixture/main.td` (a
#      `httpServe` scatter-gather loop) on the per-backend port.
#   3. Probes the port via /dev/tcp until ready.
#   4. Runs a curl loop against the server for N minutes.
#   5. Samples RSS + fd count every 30s into a CSV.
#   6. Extrapolates a 24h projection (linear fit) and flags drift
#      above 10% / hour as a FAIL signal.
#
# The 24h projection is intentionally conservative — it reports "LIKELY
# STABLE" / "DRIFT DETECTED" / "INCONCLUSIVE" verdicts, never PASS.
# Only the human-driven 24h run earns a PASS in the C26B-005 ledger.

set -euo pipefail

DURATION_MIN=30
BACKEND="interp"

while [ $# -gt 0 ]; do
    case "$1" in
        --duration-min) DURATION_MIN="$2"; shift 2 ;;
        --backend) BACKEND="$2"; shift 2 ;;
        -h|--help)
            # Print the leading docblock (file header through the end of
            # the descriptive comments). C27B-015: the docblock now spans
            # past line 30 because of the multi-backend / env-var notes,
            # so widen the slice to cover the full header.
            sed -n '1,60p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

if [ "${DURATION_MIN}" -gt 180 ]; then
    echo "--duration-min is capped at 180 (3h) inside the proxy; use the full 24h runbook for the real acceptance" >&2
    exit 1
fi

case "${BACKEND}" in
    interp) PORT=18081 ;;
    js)     PORT=18082 ;;
    native) PORT=18083 ;;
    *) echo "unknown backend: ${BACKEND} (expected interp|js|native)" >&2; exit 1 ;;
esac

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "${REPO_ROOT}"

OUTDIR="$(mktemp -d -t fastsoak.XXXXXX)"
trap 'echo "logs at ${OUTDIR}"' EXIT

CSV="${OUTDIR}/samples.csv"
LOG="${OUTDIR}/server.log"
BUILD_LOG="${OUTDIR}/build.log"
PROJECTION="${OUTDIR}/projection.txt"
FIXTURE="${OUTDIR}/main.td"

echo "timestamp_s,rss_kib,fds" > "${CSV}"

echo "==> building release binary (host, backend=${BACKEND})"
cargo build --release --bin taida

# Fixture: regenerated per-invocation inside OUTDIR (so parallel runs
# of different backends do not clobber each other's port). The runtime
# writev (scatter-gather) lives inside httpServe's response path, not
# in user code — the fixture is a steady 1-arg handler returning a
# 512 B body.
#
# Port policy: per-backend fixed port (interp=18081 / js=18082 /
# native=18083), all below Linux ip_local_port_range.min = 32768 so
# the kernel will not reassign them to ephemeral sockets mid-bind.
# Using fixed ports avoids a `getsockname` round-trip the shell proxy
# cannot express today — tracked as C27B-014 for the port-0 flow.
FIXTURE_MAX_REQS=1000000000
cat > "${FIXTURE}" <<TAIDA
// C26B-005 scatter-gather soak fixture. Kept small so it is easy to
// audit — each request returns a 512 B body which exercises the
// runtime's scatter-gather (writev) response path without dominating
// the runtime budget. Regenerated by scripts/soak/fast-soak-proxy.sh.
>>> taida-lang/net => @(httpServe)

handler req =
  @(status <= 200, headers <= @[@(name <= "content-type", value <= "text/plain")], body <= Repeat["x", 512]())
=> :@(status: Int, headers: @[@(name: Str, value: Str)], body: Str)

asyncResult <= httpServe(${PORT}, handler, ${FIXTURE_MAX_REQS})
asyncResult ]=> result
result ]=> r
stdout(r.ok)
stdout(r.requests)
TAIDA

# Dispatch per backend. All three paths produce a long-running server
# process whose PID is captured in SERVER_PID so the sampling loop can
# read /proc/<pid>/status.
case "${BACKEND}" in
    interp)
        echo "==> launching interpreter server on port ${PORT} (log: ${LOG})"
        "${REPO_ROOT}/target/release/taida" "${FIXTURE}" > "${LOG}" 2>&1 &
        SERVER_PID=$!
        ;;
    js)
        if ! command -v node >/dev/null 2>&1; then
            echo "node is required for --backend js but was not found on PATH" >&2
            exit 1
        fi
        BUILT_JS="${OUTDIR}/main.mjs"
        echo "==> compiling JS artifact (log: ${BUILD_LOG})"
        if ! "${REPO_ROOT}/target/release/taida" build --target js "${FIXTURE}" -o "${BUILT_JS}" > "${BUILD_LOG}" 2>&1; then
            echo "taida build --target js failed:" >&2
            cat "${BUILD_LOG}" >&2
            exit 2
        fi
        echo "==> launching node runtime on port ${PORT} (log: ${LOG})"
        node "${BUILT_JS}" > "${LOG}" 2>&1 &
        SERVER_PID=$!
        ;;
    native)
        BUILT_NATIVE="${OUTDIR}/main"
        echo "==> compiling native artifact (log: ${BUILD_LOG})"
        if ! "${REPO_ROOT}/target/release/taida" build --target native "${FIXTURE}" -o "${BUILT_NATIVE}" > "${BUILD_LOG}" 2>&1; then
            echo "taida build --target native failed:" >&2
            cat "${BUILD_LOG}" >&2
            exit 2
        fi
        echo "==> launching native binary on port ${PORT} (log: ${LOG})"
        "${BUILT_NATIVE}" > "${LOG}" 2>&1 &
        SERVER_PID=$!
        ;;
esac
trap 'kill ${SERVER_PID} 2>/dev/null || true; echo "logs at ${OUTDIR}"' EXIT

# Wait for bind. The interpreter does not emit a "listening on" line
# today (see .dev/C27_BLOCKERS.md::C27B-014), so we probe the port
# directly with a quick TCP connect.
READY=0
for _ in $(seq 1 60); do
    if (exec 3<>/dev/tcp/127.0.0.1/${PORT}) 2>/dev/null; then
        exec 3>&- 3<&-
        READY=1
        break
    fi
    # Fail fast if the server already died (parse error etc.).
    if ! kill -0 "${SERVER_PID}" 2>/dev/null; then
        echo "server exited before bind" >&2
        cat "${LOG}" >&2
        exit 2
    fi
    sleep 0.5
done
if [ "${READY}" -ne 1 ]; then
    echo "server did not bind 127.0.0.1:${PORT} within 30s" >&2
    cat "${LOG}" >&2
    exit 2
fi
echo "  server listening on 127.0.0.1:${PORT} (pid=${SERVER_PID})"

# Load generator: curl loop. wrk would be faster but is not always
# present; the purpose is steady scatter-gather traffic, not peak
# throughput. Real acceptance lives in wrk / h2load under the
# 24h runbook.
(
    while true; do
        curl -sS "http://127.0.0.1:${PORT}/" > /dev/null || true
    done
) &
LOAD_PID=$!
trap 'kill ${LOAD_PID} ${SERVER_PID} 2>/dev/null || true; echo "logs at ${OUTDIR}"' EXIT

END_TS=$(( $(date +%s) + DURATION_MIN * 60 ))
echo "==> sampling for ${DURATION_MIN} minutes"
while [ "$(date +%s)" -lt "${END_TS}" ]; do
    TS=$(date +%s)
    if RSS=$(awk '/^VmRSS:/ {print $2}' "/proc/${SERVER_PID}/status" 2>/dev/null); then
        FDS=$(ls "/proc/${SERVER_PID}/fd" 2>/dev/null | wc -l)
        echo "${TS},${RSS:-0},${FDS:-0}" >> "${CSV}"
    else
        echo "server disappeared (pid ${SERVER_PID})" >&2
        exit 3
    fi
    sleep 30
done

kill ${LOAD_PID} ${SERVER_PID} 2>/dev/null || true

# Linear projection to 24h. This intentionally avoids any statistics
# package dependency: awk is enough for a "drift > threshold" signal.
awk -v dur_min="${DURATION_MIN}" -F, 'NR > 1 {
    if (NR == 2) { t0 = $1; rss0 = $2; fd0 = $3 }
    tn = $1; rssn = $2; fdn = $3
}
END {
    if (!t0) { print "INCONCLUSIVE: no samples"; exit 0 }
    dt_hours = (tn - t0) / 3600.0
    if (dt_hours <= 0) { print "INCONCLUSIVE: dt too small"; exit 0 }
    rss_rate_per_hour = (rssn - rss0) / dt_hours
    fd_rate_per_hour = (fdn - fd0) / dt_hours
    rss_24h_proj = rss0 + rss_rate_per_hour * 24
    fd_24h_proj = fd0 + fd_rate_per_hour * 24
    drift_pct = (rss_rate_per_hour / rss0) * 100
    printf "fast-soak proxy %d min\n", dur_min
    printf "  RSS start: %d KiB, end: %d KiB, rate: %.1f KiB/h, 24h proj: %.0f KiB (%.1f%%/h)\n", rss0, rssn, rss_rate_per_hour, rss_24h_proj, drift_pct
    printf "  FD  start: %d, end: %d, rate: %.2f/h, 24h proj: %.0f\n", fd0, fdn, fd_rate_per_hour, fd_24h_proj
    if (drift_pct > 10.0 || fd_rate_per_hour > 5.0) {
        print "VERDICT: DRIFT DETECTED (24h soak will almost certainly fail; fix before investing the real 24h run)"
        exit 4
    }
    print "VERDICT: LIKELY STABLE (proxy PASS does not close C26B-005; run the full 24h soak per `.dev/C26_SOAK_RUNBOOK.md`)"
}' "${CSV}" | tee "${PROJECTION}"

echo "logs, samples, and projection at ${OUTDIR}"
