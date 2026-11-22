#!/usr/bin/env bash
# Builds and signs a Debian APT repository from built .deb files.
#
#   packaging/apt/build-repo.sh [repo-dir] [deb-dir ...]
#
# Defaults: repo-dir = target/apt-repo, deb-dir = target/debian-dist.
# Every existing file in the pool is kept, so old versions stay installable.
# The signing key comes from GNUPGHOME/GPG_KEY_ID; see packaging/apt/README.md.
set -euo pipefail

repo=${1:-target/apt-repo}
shift || true
mkdir -p "$repo"
repo=$(cd "$repo" && pwd)
if [ "$#" -eq 0 ]; then
    set -- target/debian-dist
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
dist="$repo/dists/stable"
pool="$repo/pool/main"
mkdir -p "$pool" "$dist"

for dir in "$@"; do
    [ -d "$dir" ] || { echo "no such directory: $dir" >&2; exit 1; }
    while IFS= read -r -d '' deb; do
        package=$(basename "$deb" | cut -d_ -f1)
        mkdir -p "$pool/f/$package"
        cp -p "$deb" "$pool/f/$package/"
    done < <(find "$dir" -maxdepth 1 -name '*.deb' -print0)
done

# A version removed from the input dirs is not removed from the pool: the
# repository keeps every version it has ever shipped. The scan recurses into
# the per-package subdirectories, so the 0.1.x ferrentd files stay visible.
mapfile -t debs < <(find "$pool" -type f -name '*.deb' | sort)
if [ "${#debs[@]}" -eq 0 ]; then
    echo "no .deb files in $pool" >&2
    exit 1
fi

arches=$(for deb in "${debs[@]}"; do dpkg-deb -f "$deb" Architecture; done | sort -u)
for arch in $arches; do
    mkdir -p "$dist/main/binary-$arch"
    # Relative to the repo root: the Filename field must work from the URL.
    (cd "$repo" && dpkg-scanpackages -m -a "$arch" pool/main /dev/null) \
        > "$dist/main/binary-$arch/Packages"
    gzip -9n -c "$dist/main/binary-$arch/Packages" > "$dist/main/binary-$arch/Packages.gz"
done

cat > "$repo/apt-ftparchive.conf" <<EOF
APT::FTPArchive::Release::Origin "Ferrent";
APT::FTPArchive::Release::Label "Ferrent";
APT::FTPArchive::Release::Suite "stable";
APT::FTPArchive::Release::Codename "stable";
APT::FTPArchive::Release::Architectures "$(echo $arches | tr ' ' ' ')";
APT::FTPArchive::Release::Components "main";
APT::FTPArchive::Release::Description "Ferrent BitTorrent daemon packages";
EOF

apt-ftparchive -c "$repo/apt-ftparchive.conf" release "$dist" > "$dist/Release"

key_args=(gpg --batch --yes)
if [ -n "${GPG_KEY_ID:-}" ]; then
    key_args+=(--local-user "$GPG_KEY_ID")
fi
# The signing key is passphrase-protected; the agent cache is not available on
# CI runners, so the passphrase comes from a file. Locally the dedicated
# keyring keeps it next to GNUPGHOME (see packaging/apt/README.md).
if [ -z "${GPG_PASSPHRASE_FILE:-}" ] && [ -f "${GNUPGHOME:-$HOME/.gnupg}/../passphrase" ]; then
    GPG_PASSPHRASE_FILE="${GNUPGHOME:-$HOME/.gnupg}/../passphrase"
fi
if [ -n "${GPG_PASSPHRASE_FILE:-}" ]; then
    key_args+=(--pinentry-mode loopback --passphrase-file "$GPG_PASSPHRASE_FILE")
fi
"${key_args[@]}" --armor --detach-sign -o "$dist/Release.gpg" "$dist/Release"
"${key_args[@]}" --clearsign -o "$dist/InRelease" "$dist/Release"

if command -v gpgv >/dev/null; then
    gpgv --keyring "$root/packaging/apt/keyring/ferrent.gpg" "$dist/InRelease" 2>/dev/null \
        || { echo "the repository signature does not match packaging/apt/keyring/ferrent.gpg" >&2; exit 1; }
fi

echo "== repository at $repo"
for arch in $arches; do
    echo "   binary-$arch:"
    awk '/^Package:/{p=$2} /^Version:/{printf "     %s %s\n", p, $2}' \
        "$dist/main/binary-$arch/Packages"
done
