#!/usr/bin/env bash
# Publishes the built .deb files to the GitLab Generic Package Registry, so the
# versions show up under Packages & Registries in the project UI.
#
#   packaging/apt/publish-gitlab.sh [deb-dir]
#
# Environment: CI_API_V4_URL + CI_PROJECT_ID + CI_JOB_TOKEN inside CI, or
# GITLAB_URL/GITLAB_PROJECT_ID/GITLAB_TOKEN locally.
#
# Note: the Debian Package Registry API routes (/debian_distributions) are not
# mounted on our GitLab (19.5.5) - measured 404 - so this uses the generic
# registry. Apt installs keep using the signed repository built by
# packaging/apt/build-repo.sh; this registry is the version history in the UI.
#
# The upload itself lives in publish-lib.sh; the AppImage publisher uses the
# same helper instead of duplicating the curl call.
set -euo pipefail

debdir=${1:-target/debian-dist}
# shellcheck source=packaging/apt/publish-lib.sh
. "$(dirname "$0")/publish-lib.sh"

shopt -s nullglob
debs=("$debdir"/*.deb)
[ "${#debs[@]}" -gt 0 ] || { echo "no .deb files in $debdir" >&2; exit 1; }

for deb in "${debs[@]}"; do
    file=$(basename "$deb")
    # ferrent_0.1.2-1_amd64.deb / ferrent-gui_0.1.2-1_amd64.deb
    package=$(printf '%s' "$file" | cut -d_ -f1)
    version=$(printf '%s' "$file" | sed -E 's/^[^_]+_([^_]+)_.*/\1/')
    [ -n "$package" ] && [ -n "$version" ] || { echo "cannot read package/version from $file" >&2; exit 1; }
    # GitLab rejects Debian's "~rc1" marker in a package version
    # ("package_version is invalid"); the file name keeps it, the path does not.
    upload_generic "$deb" "$package" "$(printf '%s' "$version" | tr '~' '-')"
done

echo "== packages in the registry:"
curl -sS "${auth[@]}" "$api/projects/$project/packages?per_page=20" \
    | sed 's/},{/}\n{/g' \
    | sed -n 's/.*"version":"\([^"]*\)".*"package_type":"\([^"]*\)".*/   \2 \1/p'
