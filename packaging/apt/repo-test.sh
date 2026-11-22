#!/usr/bin/env bash
# End-to-end check of the APT repository: builds a signed repository from the
# .deb files, then installs the key + DEB822 source in a fresh Debian container
# and installs, downgrades and upgrades ferrent through apt.
#
#   packaging/apt/repo-test.sh [deb-dir]
#
# Needs a signing key in GNUPGHOME (GPG_KEY_ID optional), exactly like
# build-repo.sh.
set -euo pipefail

debdir=${1:-target/debian-dist}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d)
name=ferrent-apt-test
docker rm -f "$name" >/dev/null 2>&1 || true
trap 'docker rm -f "$name" >/dev/null 2>&1 || true; rm -rf "$work"' EXIT

mkdir -p "$work/debs"
find "$debdir" -maxdepth 1 -name '*.deb' -exec cp -p {} "$work/debs/" \;
first_amd64=$(find "$work/debs" -name 'ferrent_*_amd64.deb' | sort | head -1)
[ -n "$first_amd64" ] || { echo "no ferrent amd64 .deb in $debdir" >&2; exit 1; }
real_version=$(dpkg-deb -f "$first_amd64" Version)
[ -n "$real_version" ] || { echo "cannot read the version of $first_amd64" >&2; exit 1; }

# A previous version proves the pool keeps history between releases.
prev=$(mktemp -d)
dpkg-deb -R "$first_amd64" "$prev"
sed -i 's/^Version: .*/Version: 0.0.9-1/' "$prev/DEBIAN/control"
dpkg-deb -b "$prev" "$work/debs/ferrent_0.0.9-1_amd64.deb" >/dev/null 2>&1
rm -rf "$prev"

"$root/packaging/apt/build-repo.sh" "$work/repo" "$work/debs"

docker run -d --name "$name" debian:12 sleep infinity >/dev/null
docker exec "$name" bash -c 'apt-get update -qq && apt-get install -y -qq ca-certificates >/dev/null'
docker cp "$work/repo/." "$name:/repo" >/dev/null
docker cp "$root/packaging/apt/keyring/ferrent.gpg" "$name:/etc/apt/keyrings/ferrent.gpg" >/dev/null
docker exec -e REAL_VERSION="$real_version" "$name" bash -c '
set -uo pipefail
fail=0
expect() { if [ "$2" = "$3" ]; then echo "PASS  $1"; else echo "FAIL  $1 (expected \"$2\", got \"$3\")"; fail=1; fi; }

cat > /etc/apt/sources.list.d/ferrent.sources <<EOF
Types: deb
URIs: file:/repo
Suites: stable
Components: main
Architectures: amd64
Signed-By: /etc/apt/keyrings/ferrent.gpg
EOF

out=$(apt-get update 2>&1) || { echo "$out"; expect "apt update" ok failed; }
expect "apt update without warnings" 0 "$(printf "%s\n" "$out" | grep -cE "^(W|E):")"
expect "both versions listed" yes \
    "$(apt-cache madison ferrent | grep -c "0.0.9-1" >/dev/null && apt-cache madison ferrent | grep -c "$REAL_VERSION" >/dev/null && echo yes || echo no)"
expect "candidate is the newest" "$REAL_VERSION" "$(apt-cache policy ferrent | awk "/Candidate:/{print \$2}")"

apt-get install -y --allow-downgrades ferrent=0.0.9-1 >/tmp/install.log 2>&1 \
    || { cat /tmp/install.log; expect "installs the previous version" ok failed; }
expect "installs the previous version" 0.0.9-1 "$(dpkg-query -W -f="\${Version}" ferrent)"
apt-get install -y ferrent >/tmp/upgrade.log 2>&1 \
    || { cat /tmp/upgrade.log; expect "upgrades to the newest" ok failed; }
expect "upgrades to the newest" "$REAL_VERSION" "$(dpkg-query -W -f="\${Version}" ferrent)"

mv /etc/apt/keyrings/ferrent.gpg /tmp/key.bak
rm -rf /var/lib/apt/lists/*
if apt-get update >/dev/null 2>&1; then
    expect "update fails without the key" failed ok
else
    expect "update fails without the key" ok ok
fi
mv /tmp/key.bak /etc/apt/keyrings/ferrent.gpg

exit $fail
' || exit 1

echo "== apt repository checks: PASS"
