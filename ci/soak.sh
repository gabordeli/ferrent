#!/usr/bin/env bash
# Sample a running ferrentd for the M2-T4 soak. Usage:
#   soak.sh PID SECONDS INTERVAL OUT [SERVER CA TOKEN]
set -euo pipefail

pid=${1:?pid}
duration=${2:-86400}
interval=${3:-60}
out=${4:-/tmp/ferrent-soak.csv}
server=${5:-https://localhost:50133}
ca=${6:-/tmp/ferrent-mvp-ready/tls-cert.pem}
token=${7:-secret}
cli=${CLI:-./target/release/ferrent-cli}

printf 'timestamp,rss_kb,cpu_pct,total,list_ms\n' >"$out"
start=$(date +%s)
sample=0
while :; do
    now=$(date +%s)
    if ! kill -0 "$pid" 2>/dev/null; then
        printf 'crash at %s\n' "$(date -Is)" >>"$out"
        exit 1
    fi
    rss=$(awk '/VmRSS:/{print $2}' "/proc/$pid/status")
    cpu=$(ps -o %cpu= -p "$pid" | tr -d ' ')
    total=""
    list_ms=""
    if (( sample % 10 == 0 )); then
        t0=$(date +%s%N)
        total=$("$cli" --server "$server" --ca "$ca" --token "$token" --json list --limit 1 2>/dev/null | jq -r .total)
        t1=$(date +%s%N)
        list_ms=$(( (t1 - t0) / 1000000 ))
    fi
    printf '%s,%s,%s,%s,%s\n' "$(date -Is)" "$rss" "$cpu" "$total" "$list_ms" >>"$out"
    sample=$((sample + 1))
    if (( now - start >= duration )); then
        printf 'soak complete at %s\n' "$(date -Is)" >>"$out"
        exit 0
    fi
    sleep "$interval"
done
