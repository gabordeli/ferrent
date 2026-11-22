#!/usr/bin/env bash
# Shared GitLab Generic Package Registry helpers.
#
# Source this from a publisher: it resolves the API endpoint, sets the `auth`
# array and defines upload_generic <file> <package> <version>.
#
# Environment: CI_API_V4_URL + CI_PROJECT_ID + CI_JOB_TOKEN inside CI, or
# GITLAB_URL/GITLAB_PROJECT_ID/GITLAB_TOKEN locally.
set -euo pipefail

api=${CI_API_V4_URL:-${GITLAB_URL:-https://cirice-gitlab.ddns.net}/api/v4}
project=${CI_PROJECT_ID:-${GITLAB_PROJECT_ID:?set GITLAB_PROJECT_ID}}

if [ -n "${CI_JOB_TOKEN:-}" ]; then
    auth=(--header "JOB-TOKEN: $CI_JOB_TOKEN")
    who="job token"
else
    : "${GITLAB_TOKEN:?set GITLAB_TOKEN or run inside CI}"
    auth=(--header "PRIVATE-TOKEN: $GITLAB_TOKEN")
    who="personal token"
fi

# upload_generic <file> <package-name> <version>
upload_generic() {
    local file=$1 package=$2 version=$3
    local name url code
    name=$(basename "$file")
    url="$api/projects/$project/packages/generic/$package/$version/$name"
    code=$(curl -sS -o /tmp/ferrent-publish.json -w '%{http_code}' \
        "${auth[@]}" --upload-file "$file" "$url")
    case "$code" in
    200 | 201) echo "published $name ($who) as $package/$version" ;;
    *)
        cat /tmp/ferrent-publish.json >&2
        echo "upload failed: $name (HTTP $code)" >&2
        return 1
        ;;
    esac
}
