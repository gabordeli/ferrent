#!/usr/bin/env bash
# Smoke-tests an arm64 .deb without arm64 hardware: the binary is run under
# qemu-user inside an amd64 container, with the arm64 runtime libraries from
# Debian multiarch.
#
#   packaging/debian/qemu-smoke.sh [deb]
set -euo pipefail

deb=${1:-target/debian-dist/ferrentd_0.1.0-1_arm64.deb}
[ -f "$deb" ] || { echo "missing $deb" >&2; exit 1; }
name=ferrent-qemu-smoke
docker rm -f "$name" >/dev/null 2>&1 || true
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT

docker run -d --name "$name" debian:12 sleep infinity >/dev/null
docker exec "$name" bash -c '
set -e
dpkg --add-architecture arm64
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    qemu-user-static libc6:arm64 libssl3:arm64 libstdc++6:arm64 zlib1g:arm64 >/dev/null
' 
docker cp "$deb" "$name:/root/pkg.deb" >/dev/null
docker exec "$name" bash -c '
set -e
dpkg-deb -x /root/pkg.deb /opt/pkg
mkdir -p /etc/ferrent /root/smoke
head -c 24 /dev/urandom | base64 > /etc/ferrent/token
printf "data_dir = \"/root/smoke\"\nbind = \"127.0.0.1:50161\"\ntoken_file = \"/etc/ferrent/token\"\n" \
    > /root/smoke/config.toml
FERRENT_CONFIG=/root/smoke/config.toml timeout 10 \
    qemu-aarch64-static /opt/pkg/usr/bin/ferrentd > /root/smoke/run.log 2>&1 || true
grep -q "ferrentd listening on https://127.0.0.1:50161" /root/smoke/run.log \
    || { cat /root/smoke/run.log; exit 1; }
grep -q "TLS fingerprint:" /root/smoke/run.log
grep -q "saved fastresume" /root/smoke/run.log
test -s /root/smoke/state.redb
echo "PASS  arm64 binary starts under qemu, opens TLS and persists state"
grep -E "startup_ms" /root/smoke/run.log
'
