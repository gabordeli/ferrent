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
set -euo pipefail

debdir=${1:-target/debian-dist}
api=${CI_API_V4_URL:-${GITLAB_URL:-https://cirice-gitlab.ddns.net}/api/v4}
project=${CI_PROJECT_ID:-${GITLAB_PROJECT_ID:?set GITLAB_PROJECT_ID}}
package=ferrentd

if [ -n "${CI_JOB_TOKEN:-}" ]; then
    auth=(--header "JOB-TOKEN: $CI_JOB_TOKEN")
    who="job token"
else
    : "${GITLAB_TOKEN:?set GITLAB_TOKEN or run inside CI}"
    auth=(--header "PRIVATE-TOKEN: $GITLAB_TOKEN")
    who="personal token"
fi

shopt -s nullglob
debs=("$debdir"/*.deb)
[ "${#debs[@]}" -gt 0 ] || { echo "no .deb files in $debdir" >&2; exit 1; }

for deb in "${debs[@]}"; do
    file=$(basename "$deb")
    version=$(printf '%s' "$file" | sed -E 's/^[^_]+_([^_]+)_.*/\1/')
    [ -n "$version" ] || { echo "cannot read the version from $file" >&2; exit 1; }
    url="$api/projects/$project/packages/generic/$package/$version/$file"
    code=$(curl -sS -o /tmp/ferrent-publish.json -w '%{http_code}' \
        "${auth[@]}" --upload-file "$deb" "$url")
    case "$code" in
    200 | 201) echo "published $file ($who) as $package/$version" ;;
    *)
        cat /tmp/ferrent-publish.json >&2
        echo "upload failed: $file (HTTP $code)" >&2
        exit 1
        ;;
    esac
done

echo "== packages in the registry:"
curl -sS "${auth[@]}" "$api/projects/$project/packages?per_page=20" \
    | sed 's/},{/}\n{/g' \
    | sed -n 's/.*"version":"\([^"]*\)".*"package_type":"\([^"]*\)".*/   \2 \1/p'
