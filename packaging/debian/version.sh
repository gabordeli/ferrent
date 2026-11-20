#!/usr/bin/env bash
# Single source of truth for package versions: Cargo.toml.
#
#   packaging/debian/version.sh                # .deb version from Cargo.toml
#   packaging/debian/version.sh v0.1.0         # validate the tag, print the .deb version
#   packaging/debian/version.sh --cargo        # raw Cargo version
#   packaging/debian/version.sh --self-test
#
# A pre-release suffix is mapped to Debian's "~" so that 0.1.0-rc1 sorts
# before 0.1.0 (dpkg compares "~" as older than the end of the string).
# DEB_REVISION overrides the Debian revision (default 1).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

cargo_version() {
    sed -n '/^\[workspace\.package\]/,/^\[/p' "$root/Cargo.toml" \
        | sed -n 's/^version = "\(.*\)"/\1/p' | head -1
}

deb_upstream() { printf '%s' "${1/-/\~}"; }
deb_version() { printf '%s-%s\n' "$(deb_upstream "$1")" "${DEB_REVISION:-1}"; }

self_test() {
    failed=0
    check() { # expected actual label
        if [ "$1" = "$2" ]; then echo "PASS  $3"; else echo "FAIL  $3 (expected $1, got $2)"; failed=1; fi
    }
    check 0.1.0-1 "$(deb_version 0.1.0)" "release version"
    check "0.1.0~rc1" "$(deb_upstream 0.1.0-rc1)" "pre-release uses ~"
    check "0.1.0~rc.1" "$(deb_upstream 0.1.0-rc.1)" "dotted pre-release"
    check 0.1.0-2 "$(DEB_REVISION=2 deb_version 0.1.0)" "revision override"
    [ -n "$(cargo_version)" ] || { echo "FAIL  Cargo.toml version not found"; failed=1; }
    [ "$failed" -eq 0 ] && echo "self-test: PASS" || exit 1
}

case "${1:-}" in
--self-test) self_test ;;
--cargo) cargo_version ;;
"")
    deb_version "$(cargo_version)"
    ;;
*)
    tag=$1
    upstream=${tag#v}
    if [ "$upstream" = "$tag" ]; then
        echo "tag must start with 'v': $tag" >&2
        exit 1
    fi
    want=$(cargo_version)
    if [ "$upstream" != "$want" ]; then
        echo "tag $tag does not match the Cargo.toml version $want" >&2
        exit 1
    fi
    deb_version "$want"
    ;;
esac
