#!/usr/bin/env bash
# Smoke test for a built AppImage: install the daemon from the ferrent .deb,
# start it, point a gui.toml profile at it, and run the AppImage headlessly.
#
#   packaging/appimage/smoke.sh <appimage> <ferrent.deb>
#
# Meant to run inside a container as root; the caller installs the system
# packages (xvfb, xauth, and whatever the .deb's Depends need). The GUI prints
# `rows=<n>` only after it connected, so that line is the assertion: it proves
# the TLS profile works from the bundled GTK/libs, not just that the binary
# starts.
set -euo pipefail

appimage=$(realpath "$1")
deb=$(realpath "$2")

# apt (not dpkg -i) resolves the package's Depends inside the minimal image.
apt-get install -y --no-install-recommends "$deb" >/dev/null
install -d /var/lib/ferrent/downloads

FERRENT_CONFIG=/etc/ferrent/config.toml ferrentd >/tmp/ferrentd.log 2>&1 &
daemon=$!
trap 'kill "$daemon" 2>/dev/null || true' EXIT

for _ in $(seq 1 120); do
    grep -q "listening on" /tmp/ferrentd.log && break
    kill -0 "$daemon" 2>/dev/null || { cat /tmp/ferrentd.log >&2; exit 1; }
    sleep 0.5
done
grep -q "listening on" /tmp/ferrentd.log || { cat /tmp/ferrentd.log >&2; exit 1; }

# The daemon prints "ferrentd listening on https://127.0.0.1:50051".
server=$(sed -n 's/^.*listening on //p' /tmp/ferrentd.log | head -1)
# The certificate carries a localhost SAN, an IP SAN is not in it, so the TLS
# verification in the GUI only passes with the host name (same lesson as the
# perf harness).
server=${server/127.0.0.1/localhost}
fingerprint=$(sed -n 's/^TLS fingerprint: //p' /tmp/ferrentd.log | head -1)
ca=$(sed -n 's/^CA certificate: //p' /tmp/ferrentd.log | head -1)
token=$(cat /etc/ferrent/token)
[ -n "$server" ] && [ -n "$fingerprint" ] && [ -n "$ca" ] || {
    echo "the daemon log has no server/fingerprint/CA lines" >&2
    cat /tmp/ferrentd.log >&2
    exit 1
}

export HOME=${HOME:-/root}
install -d "$HOME/.config/ferrent"
cat > "$HOME/.config/ferrent/gui.toml" <<EOF
active = "smoke"

[[profiles]]
name = "smoke"
server = "$server"
ca = "$ca"
token = "$token"
fingerprint = "$fingerprint"
EOF

echo "== daemon: $server"
# Not under `set -e`: a timeout is a smoke failure, and the log has to be shown
# either way (the GUI's stderr is where a missing library shows up).
set +e
timeout "${SMOKE_TIMEOUT:-90}" xvfb-run -a \
    env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 ICED_BACKEND=tiny-skia \
    "$appimage" --appimage-extract-and-run --measure >/tmp/ferrent-gui.log 2>&1
status=$?
set -e
cat /tmp/ferrent-gui.log

grep -q '^rows=' /tmp/ferrent-gui.log || {
    echo "the GUI never reported rows= (exit $status); it did not reach the daemon" >&2
    exit 1
}
echo "== appimage smoke: $(grep '^rows=' /tmp/ferrent-gui.log)"
