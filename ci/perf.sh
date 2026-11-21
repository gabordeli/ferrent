#!/usr/bin/env bash
# Measures restore time, RSS and list latency against a fresh daemon and
# compares the result with ci/perf-baseline.json.
#
# Usage: ci/perf.sh [torrents] [tolerance-percent]
# Exits 1 when any metric regressed beyond the tolerance.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="${HOME}/.cargo/bin:${PATH}"
TORRENTS=${1:-2000}
TOLERANCE=${2:-10}
BASE=${ROOT}/ci/perf-baseline.json
DATA=$(mktemp -d /tmp/ferrent-perf.XXXXXX)
PORT=${PERF_PORT:-50711}
URL="https://localhost:${PORT}"
TOKEN=perf-token
OUT=${DATA}/result.json
RESULT=${ROOT}/perf-result.json

cleanup() {
    if [ -n "${DAEMON_PID:-}" ] && kill -0 "${DAEMON_PID}" 2>/dev/null; then
        kill "${DAEMON_PID}" 2>/dev/null || true
        for _ in $(seq 1 40); do
            kill -0 "${DAEMON_PID}" 2>/dev/null || break
            sleep 0.25
        done
    fi
    rm -rf "${DATA}"
}
trap cleanup EXIT

echo "preparing ${TORRENTS} torrents in ${DATA}"
cargo run --quiet --release -p ferrent-daemon --features libtorrent \
    --example mvp_prepare -- --torrents "${TORRENTS}" --data-dir "${DATA}/state" >/dev/null

FERRENT_DATA_DIR=${DATA}/state FERRENT_BIND=127.0.0.1:${PORT} FERRENT_TOKEN=${TOKEN} \
    "${ROOT}/target/release/ferrentd" >"${DATA}/daemon.log" 2>&1 &
DAEMON_PID=$!

for _ in $(seq 1 160); do
    if grep -q "listening on" "${DATA}/daemon.log" 2>/dev/null; then
        break
    fi
    sleep 0.25
done
grep -q "listening on" "${DATA}/daemon.log" || {
    echo "daemon did not start" >&2
    tail -5 "${DATA}/daemon.log" >&2
    exit 1
}

STARTUP_MS=$(sed -n 's/^startup_ms=//p' "${DATA}/daemon.log" | head -1)
RESTORED=$(sed -n 's/^restored_torrents=//p' "${DATA}/daemon.log" | head -1)
RSS_KB=$(awk '/VmRSS/{print $2}' "/proc/${DAEMON_PID}/status")

cargo run --quiet --release -p ferrent-daemon --example perf_probe -- \
    "${URL}" "${DATA}/state/tls-cert.pem" "${TOKEN}" 30 >"${DATA}/probe.json"
PROBE=$(cat "${DATA}/probe.json")

python3 - "$OUT" "$STARTUP_MS" "$RSS_KB" "$RESTORED" "$PROBE" <<'PY'
import json, sys
out, startup, rss, restored, probe = sys.argv[1:6]
data = json.loads(probe)
data["startup_ms"] = float(startup or 0)
data["rss_kb"] = int(rss or 0)
data["restored"] = int(restored or 0)
json.dump(data, open(out, "w"), indent=2)
print(json.dumps(data, indent=2))
PY

cp "${OUT}" "${RESULT}" 2>/dev/null || true

if [ ! -f "${BASE}" ]; then
    echo "no baseline at ${BASE}; writing the current run as the baseline"
    cp "${OUT}" "${BASE}"
    cp "${OUT}" "${RESULT}"
    exit 0
fi

python3 - "${BASE}" "${OUT}" "${TOLERANCE}" <<'PY'
import json, sys
base, current, tolerance = sys.argv[1], sys.argv[2], float(sys.argv[3])
base = json.load(open(base))
cur = json.load(open(current))
failed = []
for key in ("startup_ms", "list_p95_ms", "rss_kb"):
    if key not in base or key not in cur or base[key] == 0:
        continue
    allowed = base[key] * (1.0 + tolerance / 100.0)
    if cur[key] > allowed:
        failed.append(f"{key}: {cur[key]:.1f} > {allowed:.1f} ({base[key]:.1f} +{tolerance:.0f}%)")
    else:
        print(f"ok   {key}: {cur[key]:.1f} (baseline {base[key]:.1f})")
if failed:
    print("REGRESSION:\n  " + "\n  ".join(failed))
    sys.exit(1)
PY
