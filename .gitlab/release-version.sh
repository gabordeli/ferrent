#!/usr/bin/env bash
# Manual release: resolves the version (explicit RELEASE_VERSION or a bump from
# the last v* tag), bumps Cargo.toml, commits on main, tags that commit and
# publishes RELEASE_TAG/RELEASE_COMMIT/DEB_VERSION for the build jobs.
#
# Runs in a "Run pipeline" web pipeline, so GITLAB_TOKEN (api +
# write_repository) must be set as a CI variable.
set -euo pipefail

die() { echo "$*" >&2; exit 1; }
for name in GITLAB_TOKEN CI_API_V4_URL CI_PROJECT_ID CI_SERVER_HOST CI_PROJECT_PATH; do
    [ -n "${!name:-}" ] || die "missing required env: $name"
done

release_type=${RELEASE_TYPE:-patch}
case "$release_type" in *'$[['*) release_type=patch ;; esac
version=${RELEASE_VERSION:-}
case "$version" in *'$[['*) version="" ;; esac

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if [ -n "$version" ]; then
    version=${version#v}
else
    last=$(git tag -l 'v*' --sort=version:refname | tail -1)
    last=${last:-v0.0.0}
    base=${last#v}
    base=${base%%-*}
    IFS=. read -r major minor patch <<<"$base"
    major=${major:-0}; minor=${minor:-0}; patch=${patch:-0}
    case "$release_type" in
    patch) patch=$((patch + 1)) ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    major) major=$((major + 1)); minor=0; patch=0 ;;
    *) die "invalid RELEASE_TYPE: $release_type (patch|minor|major)" ;;
    esac
    version="$major.$minor.$patch"
fi

tag="v$version"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    die "tag already exists: $tag"
fi

current=$(sed -n '/^\[workspace\.package\]/,/^\[/p' Cargo.toml \
    | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
echo "== $current -> $version ($tag)"
sed -i "s/^version = \"$current\"/version = \"$version\"/" Cargo.toml
# Cargo.lock is refreshed by the build (path-dependency versions only).

git config user.name "ferrent-bot"
git config user.email "ferrent-bot@users.noreply.github.com"
remote="https://oauth2:${GITLAB_TOKEN}@${CI_SERVER_HOST}/${CI_PROJECT_PATH}.git"

# [skip ci]: the tag pipeline would rebuild what this pipeline already publishes.
if git diff --quiet -- Cargo.toml; then
    echo "Cargo.toml already at $version, tagging the current commit"
else
    git add Cargo.toml
    git commit -q -m "chore(release): $version [skip ci]"
    git push -q -o ci.skip "$remote" "HEAD:${CI_DEFAULT_BRANCH:-main}"
fi
commit=$(git rev-parse HEAD)

# The tag goes through git, so a PAT with write_repository is enough (the
# tags API additionally needs the api scope).
git tag -a "$tag" -m "Release $tag" "$commit"
git push -q -o ci.skip "$remote" "refs/tags/$tag"

mkdir -p tmp
cat >tmp/release.env <<EOF
RELEASE_TAG=$tag
RELEASE_COMMIT=$commit
DEB_VERSION=$("$root/packaging/debian/version.sh" "$tag")
EOF
echo "== released $tag at $commit"
cat tmp/release.env
