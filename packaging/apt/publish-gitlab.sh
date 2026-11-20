#!/usr/bin/env bash
# Publishes the built .deb files to the GitLab Debian Package Registry, so the
# versions show up in the project's UI and apt can install from it.
#
#   packaging/apt/publish-gitlab.sh [deb-dir]
#
# Environment:
#   CI_API_V4_URL, CI_PROJECT_ID, CI_JOB_TOKEN   (inside GitLab CI)
#   GITLAB_URL, GITLAB_PROJECT_ID, GITLAB_TOKEN  (or GITLAB_USER + token locally)
#   CODENAMES  default "bookworm trixie noble"
#   COMPONENT  default "main"
#
# The same package installs on every codename: only the registry path differs.
set -euo pipefail

debdir=${1:-target/debian-dist}
api=${CI_API_V4_URL:-${GITLAB_URL:-https://cirice-gitlab.ddns.net}/api/v4}
project=${CI_PROJECT_ID:-${GITLAB_PROJECT_ID:?set GITLAB_PROJECT_ID}}
codenames=${CODENAMES:-bookworm trixie noble}
component=${COMPONENT:-main}

if [ -n "${CI_JOB_TOKEN:-}" ]; then
    auth=(-u "gitlab-ci-token:$CI_JOB_TOKEN")
    headers=(--header "JOB-TOKEN: $CI_JOB_TOKEN")
    who="job token"
else
    : "${GITLAB_TOKEN:?set GITLAB_TOKEN or run inside CI}"
    auth=(-u "${GITLAB_USER:-gabor}:$GITLAB_TOKEN")
    headers=(--header "PRIVATE-TOKEN: $GITLAB_TOKEN")
    who="personal token"
fi

shopt -s nullglob
debs=("$debdir"/*.deb)
[ "${#debs[@]}" -gt 0 ] || { echo "no .deb files in $debdir" >&2; exit 1; }

for codename in $codenames; do
    body=$(mktemp)
    code=$(curl -sS -o "$body" -w '%{http_code}' "${headers[@]}" -X POST \
        "$api/projects/$project/debian_distributions?codename=$codename")
    case "$code" in
    201) echo "distribution $codename created" ;;
    409) echo "distribution $codename already exists" ;;
    *) cat "$body" >&2; echo "cannot create distribution $codename (HTTP $code)" >&2; exit 1 ;;
    esac
    rm -f "$body"

    for deb in "${debs[@]}"; do
        name=$(basename "$deb")
        code=$(curl -sS -o /dev/null -w '%{http_code}' "${auth[@]}" --upload-file "$deb" \
            "$api/projects/$project/packages/debian/$name?distribution=$codename&component=$component")
        case "$code" in
        200 | 201) echo "published $name -> $codename/$component" ;;
        *) echo "upload failed: $name -> $codename (HTTP $code)" >&2; exit 1 ;;
        esac
    done
done

echo "== packages in the registry ($who):"
curl -sS "${headers[@]}" "$api/projects/$project/packages?package_type=debian&per_page=100" \
    | grep -oE '"name":"[^"]+"|"version":"[^"]+"' | paste - - | sed 's/^/   /'
