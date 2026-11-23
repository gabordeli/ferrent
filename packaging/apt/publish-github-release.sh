#!/usr/bin/env bash
# Attaches the built .deb files to a GitHub release, so the mirror's
# release-apt workflow can sign and publish them without rebuilding.
#
#   packaging/apt/publish-github-release.sh <tag> [deb-dir]
#
# Environment:
#   GITHUB_TOKEN       required; Contents: read/write and Actions: read/write
#                       (the latter for the release-apt dispatch)
#   GITHUB_REPOSITORY  default gabordeli/ferrent
#   RELEASE_COMMIT     optional target_commitish when the release is created
#
# Idempotent: an asset with the same name is skipped.
set -euo pipefail

tag=${1:?usage: $0 <tag> [deb-dir]}
debdir=${2:-target/debian-dist}
repo=${GITHUB_REPOSITORY:-gabordeli/ferrent}
api=${GITHUB_API_URL:-https://api.github.com}
: "${GITHUB_TOKEN:?set GITHUB_TOKEN (Contents: read and write)}"

auth=(--header "Authorization: Bearer $GITHUB_TOKEN"
    --header "Accept: application/vnd.github+json"
    --header "X-GitHub-Api-Version: 2022-11-28")

shopt -s nullglob
debs=("$debdir"/*.deb)
[ "${#debs[@]}" -gt 0 ] || { echo "no .deb files in $debdir" >&2; exit 1; }

# Find or create the release for the tag.
code=$(curl -sS -o /tmp/gh-release.json -w '%{http_code}' \
    "${auth[@]}" "$api/repos/$repo/releases/tags/$tag")
case "$code" in
200) ;;
404)
    prerelease=false
    case "$tag" in *-rc*) prerelease=true ;; esac
    body=$(python3 - "$tag" "$prerelease" "${RELEASE_COMMIT:-}" <<'PY'
import json, sys
tag, prerelease, commit = sys.argv[1], sys.argv[2] == "true", sys.argv[3]
out = {"tag_name": tag, "name": f"Ferrent {tag}", "prerelease": prerelease}
if commit:
    out["target_commitish"] = commit
print(json.dumps(out))
PY
)
    code=$(curl -sS -o /tmp/gh-release.json -w '%{http_code}' -X POST \
        "${auth[@]}" -d "$body" "$api/repos/$repo/releases")
    case "$code" in
    200 | 201) echo "created the release $tag" ;;
    *)
        cat /tmp/gh-release.json >&2
        echo "cannot create the release $tag (HTTP $code)" >&2
        exit 1
        ;;
    esac
    ;;
*)
    cat /tmp/gh-release.json >&2
    echo "cannot read the release $tag (HTTP $code)" >&2
    exit 1
    ;;
esac

release_id=$(python3 -c 'import json; print(json.load(open("/tmp/gh-release.json"))["id"])')
curl -sS "${auth[@]}" "$api/repos/$repo/releases/$release_id/assets?per_page=100" \
    -o /tmp/gh-assets.json
uploaded=0
for deb in "${debs[@]}"; do
    file=$(basename "$deb")
    if python3 -c '
import json, sys
names = {asset["name"] for asset in json.load(open("/tmp/gh-assets.json"))}
sys.exit(0 if sys.argv[1] in names else 1)
' "$file"; then
        echo "asset exists, skipping $file"
        continue
    fi
    code=$(curl -sS -o /tmp/gh-asset.json -w '%{http_code}' -X POST \
        "${auth[@]}" \
        --header "Content-Type: application/vnd.debian.binary-package" \
        --data-binary "@$deb" \
        "https://uploads.github.com/repos/$repo/releases/$release_id/assets?name=$file")
    case "$code" in
    201) echo "uploaded $file"; uploaded=$((uploaded + 1)) ;;
    *)
        cat /tmp/gh-asset.json >&2
        echo "upload failed: $file (HTTP $code)" >&2
        exit 1
        ;;
    esac
done

echo "== ${#debs[@]} .deb asset(s) on release $tag ($uploaded uploaded now)"

# Trigger the signing workflow only once every asset is in place. The
# `release: published` event fires when the release is created, which races the
# per-file uploads (a 30 MB deb takes seconds), so the GitLab job dispatches
# the workflow explicitly. A token without Actions: write must not fail the
# release: the assets are already uploaded, only the APT refresh needs a nudge.
code=$(curl -sS -o /tmp/gh-dispatch.json -w '%{http_code}' -X POST "${auth[@]}" \
    -d "{\"ref\":\"main\",\"inputs\":{\"tag\":\"$tag\"}}" \
    "$api/repos/$repo/actions/workflows/release-apt.yml/dispatches")
case "$code" in
204) echo "release-apt dispatched for $tag" ;;
*)
    cat /tmp/gh-dispatch.json >&2
    echo "warning: cannot dispatch release-apt (HTTP $code); the token needs Actions: read and write." >&2
    echo "warning: the release assets are in place; run the release-apt workflow by hand to refresh the APT repo." >&2
    ;;
esac
