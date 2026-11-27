#!/usr/bin/env bash
# Deploy a development build to the live machines without versioning anything:
# no tag, no package install, no release pipeline.
#
#   packaging/debian/dev-deploy.sh build            # daemon + CLI, bookworm container
#   packaging/debian/dev-deploy.sh server           # build, install on 192.168.2.27, restart
#   packaging/debian/dev-deploy.sh gui              # laptop GUI from target/release, then run it
#   packaging/debian/dev-deploy.sh all              # gui + server
#   packaging/debian/dev-deploy.sh status           # what is live on the server right now
#   packaging/debian/dev-deploy.sh rollback         # back to the packaged /usr/bin/ferrentd
#
# Options:
#   --rebuild        rebuild even when the binaries match the working tree
#   --no-restart     install the binaries and the unit drop-in, do not restart
#   --build-only     (gui) build, do not launch
#   --host <target>  ssh target (default root@192.168.2.27)
#
# The packaged files stay what dpkg owns: the dev build goes to
# /usr/local/bin/ferrentd (+ /usr/local/lib/ferrent-dev/) and a systemd
# drop-in points ExecStart there. `rollback` removes the drop-in, so the
# packaged binary takes over again — no reinstall needed.
#
# The daemon restart keeps the daemon's TLS certificate (same data dir), so the
# laptop's pinned gui.toml profile keeps working. The pause state is recorded
# before the restart and re-applied afterwards: a torrent that was paused stays
# paused even if the restore re-activates it.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
dist="$root/target/dev-dist"
host=${FERRENT_DEV_HOST:-root@192.168.2.27}
command=${1:-}
rebuild=""
no_restart=""
build_only=""
if [ $# -gt 0 ]; then shift; fi

while [ $# -gt 0 ]; do
    case "$1" in
    --rebuild) rebuild=1 ;;
    --no-restart) no_restart=1 ;;
    --build-only) build_only=1 ;;
    --host)
        shift
        host=$1
        ;;
    --host=*) host=${1#--host=} ;;
    -h | --help)
        sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        echo "unknown option: $1" >&2
        exit 2
        ;;
    esac
    shift
done

log() { printf '== %s\n' "$*"; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

# Short identity of the working tree: the commit plus a hash of the dirty bits
# (tracked changes and untracked files alike), so an edited tree never reuses
# the previous build.
stamp() {
    local rev
    rev=$(git -C "$root" rev-parse --short HEAD)
    if git -C "$root" diff --quiet && git -C "$root" diff --cached --quiet &&
        [ -z "$(git -C "$root" ls-files --others --exclude-standard)" ]; then
        printf '%s' "$rev"
    else
        printf '%s+%s' "$rev" \
            "$({ git -C "$root" status --porcelain; git -C "$root" diff; } | sha256sum | cut -c1-8)"
    fi
}

do_build() {
    local want
    want=$(stamp)
    if [ -z "$rebuild" ] && [ -x "$dist/ferrentd" ] && [ "$(cat "$dist/stamp" 2>/dev/null)" = "$want" ]; then
        log "binaries already match $want (--rebuild to force a build)"
        return
    fi

    log "building $want for Debian 12 (static libtorrent, daemon + CLI only)"
    FERRENT_SKIP_GUI=1 "$root/packaging/debian/build.sh" debian:12 amd64

    local deb
    deb=$(ls -t "$root"/target/debian-dist/ferrent_*.deb | head -1)
    [ -n "$deb" ] || die "build.sh produced no .deb"
    rm -rf "$dist"
    mkdir -p "$dist/.root"
    dpkg-deb -x "$deb" "$dist/.root"
    install -m755 "$dist/.root/usr/bin/ferrentd" "$dist/ferrentd"
    install -m755 "$dist/.root/usr/bin/ferrent-cli" "$dist/ferrent-cli"
    rm -rf "$dist/.root"
    printf '%s\n' "$want" >"$dist/stamp"
    log "built $(sha256sum "$dist/ferrentd" | cut -c1-16) from $want"
}

# The CLI picks the connection up from FERRENT_SERVER/FERRENT_CA/FERRENT_TOKEN;
# setting them on the remote side keeps the token out of this process' argv.
remote() { ssh -o BatchMode=yes "$host" "export FERRENT_SERVER=https://localhost:50051 FERRENT_CA=/var/lib/ferrent/tls-cert.pem; export FERRENT_TOKEN=\$(cat /etc/ferrent/token); $*"; }

# One snapshot of the whole list drives both the counters and the pause set: it
# works against the packaged rc1 CLI too, which has no `status` subcommand yet.
snapshot() { remote "ferrent-cli list --limit 0 --json"; }

summarize() {
    python3 -c '
import json, sys
from collections import Counter
d = json.load(sys.stdin)
counts = Counter(t["state"] for t in d["torrents"])
print("   torrents={} ".format(d["total"]) + " ".join(
    "{}={}".format(k, v) for k, v in sorted(counts.items())))
'
}

paused_ids() {
    python3 -c '
import json, sys
d = json.load(sys.stdin)
print(" ".join(str(t["id"]) for t in d["torrents"] if t["state"] == "paused"))
'
}

wait_ready() {
    log "waiting for the daemon to answer"
    ssh -o BatchMode=yes "$host" '
for _ in $(seq 1 90); do
    if systemctl is-active --quiet ferrentd; then
        if FERRENT_SERVER=https://localhost:50051 \
           FERRENT_CA=/var/lib/ferrent/tls-cert.pem \
           FERRENT_TOKEN=$(cat /etc/ferrent/token) \
           ferrent-cli list --limit 1 --json >/dev/null 2>&1; then
            exit 0
        fi
    fi
    sleep 2
done
systemctl status --no-pager --lines=20 ferrentd >&2 || true
exit 1'
}

# A restart can re-activate torrents that were paused; put exactly those back.
restore_pause() {
    local before=$1 after missing
    after=$(snapshot | paused_ids)
    missing=$(comm -23 <(printf '%s\n' $before | sort) <(printf '%s\n' $after | sort))
    if [ -z "$missing" ]; then
        log "pause state intact ($(wc -w <<<"$after") paused)"
        return
    fi
    log "re-pausing $(wc -w <<<"$missing") torrent(s) the restart re-activated"
    printf '%s\n' $missing | ssh -o BatchMode=yes "$host" \
        "export FERRENT_SERVER=https://localhost:50051 FERRENT_CA=/var/lib/ferrent/tls-cert.pem; export FERRENT_TOKEN=\$(cat /etc/ferrent/token); xargs -P 8 -n 1 ferrent-cli pause"
    log "pause state now $(wc -w <<<"$(snapshot | paused_ids)") paused"
}

do_server() {
    do_build

    local snap before
    snap=$(snapshot)
    log "state before"
    printf '%s' "$snap" | summarize
    before=$(printf '%s' "$snap" | paused_ids)
    log "$(wc -w <<<"$before") paused torrent(s) recorded"

    ssh -o BatchMode=yes "$host" 'install -d -m755 /usr/local/lib/ferrent-dev'
    scp -q "$dist/ferrentd" "$dist/ferrent-cli" "$dist/stamp" \
        "$host:/usr/local/lib/ferrent-dev/"
    ssh -o BatchMode=yes "$host" bash -s <<'REMOTE'
set -euo pipefail
install -m755 /usr/local/lib/ferrent-dev/ferrentd /usr/local/bin/ferrentd
install -m755 /usr/local/lib/ferrent-dev/ferrent-cli /usr/local/bin/ferrent-cli
install -m644 /usr/local/lib/ferrent-dev/stamp /usr/local/lib/ferrent-dev/BUILD
install -d /etc/systemd/system/ferrentd.service.d
cat >/etc/systemd/system/ferrentd.service.d/dev.conf <<'CONF'
[Service]
ExecStart=
ExecStart=/usr/local/bin/ferrentd
CONF
systemctl daemon-reload
REMOTE

    local want remote_sha local_sha
    want=$(cat "$dist/stamp")
    local_sha=$(sha256sum "$dist/ferrentd" | cut -d' ' -f1)
    remote_sha=$(ssh -o BatchMode=yes "$host" 'sha256sum /usr/local/bin/ferrentd' | cut -d' ' -f1)
    [ "$remote_sha" = "$local_sha" ] ||
        die "the server runs a different binary than the one just built"
    log "installed $want ($(cut -c1-16 <<<"$remote_sha"))"

    if [ -n "$no_restart" ]; then
        log "drop-in written; restart it with: ssh $host systemctl restart ferrentd"
        return
    fi

    log "restarting ferrentd"
    ssh -o BatchMode=yes "$host" systemctl restart ferrentd
    wait_ready
    log "state after"
    snapshot | summarize
    restore_pause "$before"
    log "live build: $want (the GUI client needs a restart to reconnect)"
}

do_gui() {
    log "building ferrent-gui (release)"
    cargo build --release --manifest-path "$root/Cargo.toml" -p ferrent-gui
    if [ -n "$build_only" ]; then return; fi
    # iced opens a native Wayland window when WAYLAND_DISPLAY is set, which the
    # X11 tooling cannot see; the laptop session is X11.
    pkill -x ferrent-gui 2>/dev/null || true
    sleep 1
    log "starting $(stamp) (Ctrl-C closes it)"
    exec env -u WAYLAND_DISPLAY DISPLAY=${DISPLAY:-:0} WINIT_UNIX_BACKEND=x11 \
        "$root/target/release/ferrent-gui"
}

do_status() {
    ssh -o BatchMode=yes "$host" '
set -u
echo "unit:      $(systemctl show -p ActiveState --value ferrentd)"
echo "exec:      $(systemctl show -p ExecStart --value ferrentd | sed "s/.*path=\([^ ;]*\).*/\1/")"
if [ -f /usr/local/lib/ferrent-dev/BUILD ]; then
    echo "dev build: $(cat /usr/local/lib/ferrent-dev/BUILD)"
else
    echo "dev build: (none)"
fi
for f in /usr/bin/ferrentd /usr/local/bin/ferrentd; do
    [ -x "$f" ] && echo "sha256:    $(sha256sum "$f" | cut -c1-16)  $f"
done
exit 0'
    snapshot | summarize
}

do_rollback() {
    local before
    before=$(snapshot | paused_ids)
    ssh -o BatchMode=yes "$host" '
set -e
rm -f /etc/systemd/system/ferrentd.service.d/dev.conf
rmdir --ignore-fail-on-non-empty /etc/systemd/system/ferrentd.service.d 2>/dev/null || true
systemctl daemon-reload
systemctl restart ferrentd'
    wait_ready
    log "exec now: $(ssh -o BatchMode=yes "$host" 'systemctl show -p ExecStart --value ferrentd')"
    snapshot | summarize
    restore_pause "$before"
}

case "$command" in
build) do_build ;;
server) do_server ;;
gui) do_gui ;;
all)
    do_build
    do_server
    do_gui
    ;;
status) do_status ;;
rollback) do_rollback ;;
*)
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
