#!/usr/bin/env bash
# Publishes dist/Ferrent-<version>-<arch>.AppImage to the GitLab Generic
# Package Registry as package `ferrent-gui`.
#
#   packaging/appimage/publish.sh [dir]
#
# Environment: same as packaging/apt/publish-gitlab.sh (CI job token in CI).
set -euo pipefail

dir=${1:-dist}
# shellcheck source=packaging/apt/publish-lib.sh
. "$(dirname "$0")/../apt/publish-lib.sh"

shopt -s nullglob
images=("$dir"/Ferrent-*.AppImage)
[ "${#images[@]}" -gt 0 ] || { echo "no AppImage in $dir" >&2; exit 1; }

for image in "${images[@]}"; do
    name=$(basename "$image")
    # Ferrent-1.0.0-rc1-x86_64.AppImage -> 1.0.0-rc1
    version=${name#Ferrent-}
    version=${version%-x86_64.AppImage}
    [ -n "$version" ] && [ "$version" != "$name" ] || {
        echo "cannot read the version from $name" >&2
        exit 1
    }
    upload_generic "$image" ferrent-gui "$version"
done
