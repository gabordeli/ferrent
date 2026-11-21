#!/usr/bin/env bash
# M7-T4 acceptance run: gateway under load for an hour.
#
#   ci/web-soak.sh [SECONDS] [TORRENTS]
#
# Starts the demo daemon with N mock torrents and the gateway over HTTPS,
# holds three SSE clients and adds/removes a torrent every 30 s, then samples
# the gateway every 60 s (RSS, established SSE sockets, events per client).
#
# Everything lives under $ROOT (default /tmp/ferrent-web-soak); the run is
# disposable, the CSV is the result: $ROOT/gateway.csv.
set -euo pipefail

duration=${1:-3600}
torrents=${2:-10000}
root=${ROOT:-/tmp/ferrent-web-soak}
http_port=${HTTP_PORT:-18444}
daemon_port=${DAEMON_PORT:-50190}
password=soak
csv=$root/gateway.csv

cd "$(dirname "$0")/.."
demo=./target/release/examples/demo_server
gateway=./target/release/ferrent-web
for bin in "$demo" "$gateway"; do
    [ -x "$bin" ] || {
        echo "$bin missing; run: cargo build --release -p ferrent-web -p ferrent-gui --example demo_server" >&2
        exit 1
    }
done

rm -rf "$root"
mkdir -p "$root"
cat >"$root/web.toml" <<EOF
listen = "127.0.0.1:$http_port"
password = "$password"
read_only = false
spa_dir = "webui/dist"

[tls]
cert = "$root/web-cert.pem"
key = "$root/web-key.pem"
names = ["localhost"]

[daemon]
url = "https://localhost:$daemon_port"
ca = "$root/tls-cert.pem"
token = "secret"
EOF

cleanup() {
    kill $(jobs -p) 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

"$demo" --torrents "$torrents" --port "$daemon_port" --data-dir "$root" \
    >"$root/daemon.log" 2>&1 &
sleep 4
"$gateway" --config "$root/web.toml" >"$root/gateway.log" 2>&1 &
gateway_pid=$!

for _ in $(seq 1 50); do
    curl -fsSk "https://localhost:$http_port/health" >/dev/null 2>&1 && break
    sleep 0.2
done
curl -fsSk "https://localhost:$http_port/health" | grep -q '"status":"ok"' || {
    echo "gateway is not healthy:" >&2
    tail -20 "$root/gateway.log" >&2
    exit 1
}

# Three SSE clients, one cookie jar each; each counts the events it receives.
for client in 1 2 3; do
    jar=$root/session-$client.txt
    curl -fsSk -c "$jar" -H 'content-type: application/json' \
        -d "{\"password\":\"$password\"}" \
        "https://localhost:$http_port/api/login" >/dev/null
    : >"$root/events-$client.count"
    (
        while :; do
            curl -sNk -b "$jar" "https://localhost:$http_port/api/events" |
                while IFS= read -r line; do
                    case $line in
                    data:*) echo x >>"$root/events-$client.count" ;;
                    esac
                done
            sleep 1
        done
    ) &
    sleep 0.3
done

# Traffic: one add and one remove per interval through the API the UI uses.
(
    jar=$root/session-1.txt
    n=0
    while :; do
        id=$(curl -fsSk -b "$jar" -H 'content-type: application/json' \
            -d "{\"magnet\":\"magnet:?xt=urn:btih:$(printf '%040d' "$n")\",\"name\":\"soak-$n.bin\",\"total_bytes\":1048576}" \
            "https://localhost:$http_port/api/torrents" 2>/dev/null |
            sed -n 's/.*"id":\([0-9]*\).*/\1/p')
        sleep 30
        [ -n "$id" ] && curl -fsSk -b "$jar" -X DELETE \
            "https://localhost:$http_port/api/torrents/$id" >/dev/null 2>&1 || true
        sleep 30
        n=$((n + 1))
    done
) &

printf 'timestamp,rss_kb,sse_sockets,events_1,events_2,events_3,total\n' >"$csv"
start=$(date +%s)
while :; do
    rss=$(awk '/VmRSS:/{print $2}' "/proc/$gateway_pid/status")
    sockets=$(ss -tn state established "( sport = :$http_port )" 2>/dev/null | tail -n +2 | wc -l)
    e1=$(wc -l <"$root/events-1.count")
    e2=$(wc -l <"$root/events-2.count")
    e3=$(wc -l <"$root/events-3.count")
    total=$(curl -fsSk -b "$root/session-1.txt" \
        "https://localhost:$http_port/api/torrents?limit=1" 2>/dev/null |
        sed -n 's/.*"total":\([0-9]*\).*/\1/p')
    printf '%s,%s,%s,%s,%s,%s,%s\n' "$(date -Is)" "$rss" "$sockets" "$e1" "$e2" "$e3" "$total" >>"$csv"
    now=$(date +%s)
    (( now - start >= duration )) && break
    sleep 60
done
echo "soak complete: $csv"
