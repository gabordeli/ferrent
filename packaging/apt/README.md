# Ferrent APT repository

The `.deb` files are published from the `gh-pages` branch as a signed APT
repository (`dists/stable` + `pool/main/f/<package>`). The client side is
described in `docs/install-debian.md`.

## Signing key

| | |
|---|---|
| UID | `Ferrent APT Repository <apt@ferrent.invalid>` |
| Fingerprint | `3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431` |
| Type | Ed25519 (sign) + cv25519 (encrypt), passphrase-protected |
| Valid until | 2031-11-21 |

The previous RSA 4096 key (`DB161AB5A22473586AC52CEDD727691B9D3B58AA`) stays in
the keyring for the transition; new signatures use the Ed25519 key.

The public keyring is tracked as `packaging/apt/keyring/ferrent.gpg` (both keys)
and is also served next to the repository (`/ferrent.gpg`), so `apt` can fetch
it over HTTPS.

The private key lives **outside the repository**, in a dedicated keyring so it
does not mix with the personal one:

```bash
export GNUPGHOME="$HOME/.config/ferrent/apt/gnupg"
# armored secret key, mode 0600:
#   $GNUPGHOME/../ferrent-apt-private.asc
# passphrase, mode 0600:
#   $GNUPGHOME/../passphrase
```

The key is passphrase-protected. CI runners have no gpg-agent cache, so
`build-repo.sh` reads the passphrase from `GPG_PASSPHRASE_FILE`, and falls back
to `$GNUPGHOME/../passphrase` when the variable is unset.

GitHub secrets (repository settings → Secrets and variables → Actions):

| Secret | Value |
|---|---|
| `GPG_PRIVATE_KEY` | `base64 -w0 ~/.config/ferrent/apt/ferrent-apt-private.asc` |
| `GPG_KEY_ID` | `3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431` |
| `GPG_PASSPHRASE` | `cat ~/.config/ferrent/apt/passphrase` |

## Build and test locally

```bash
export GNUPGHOME="$HOME/.config/ferrent/apt/gnupg"
export GPG_KEY_ID=3E028C8A4521C1289117CCBE1F9CEB5C0BB1A431
export GPG_PASSPHRASE_FILE="$GNUPGHOME/../passphrase"   # optional fallback

# 1. packages (one per arch)
packaging/debian/build.sh debian:12 amd64
packaging/debian/build.sh debian:12 arm64

# 2. signed repository from target/debian-dist
packaging/apt/build-repo.sh target/apt-repo target/debian-dist

# 3. apt install / downgrade / upgrade in a fresh Debian container,
#    including the "update fails without the key" case
packaging/apt/repo-test.sh
```

`repo-test.sh` fabricates a `0.0.9-1` package next to the real one, so the
"previous version still installable" property is actually exercised.

The repository keeps every version it has ever seen: files are never removed
from `pool/`, and `dpkg-scanpackages -m` lists all of them. A version is only
dropped by deleting it from the branch by hand.

## Publishing

The canonical build runs once, on GitLab:

1. `release-deb` builds `ferrent` and `ferrent-gui` in the container, runs the
   container test, and publishes them to the GitLab generic registry;
2. `packaging/apt/publish-github-release.sh` attaches the same files to the
   GitHub release (`GITHUB_RELEASE_TOKEN` CI variable, Contents: read/write and
   Actions: read/write) and dispatches the signing workflow once the uploads
   are done.

`.github/workflows/release-apt.yml` runs on `workflow_dispatch` (the GitLab
job or manually with a tag):

1. downloads the `.deb` assets of the release,
2. checks out `gh-pages` (or starts an empty orphan branch),
3. regenerates `dists/stable` with `apt-ftparchive`, signs `InRelease` and
   `Release.gpg`, copies the public key to `/ferrent.gpg`,
4. commits and pushes `gh-pages`.

GitHub Pages must serve that branch: **Settings → Pages → Source: Deploy from a
branch → `gh-pages` / `/`**. The repository URL is then
`https://<owner>.github.io/ferrent/`; adjust `URIs:` in
`packaging/apt/ferrent.sources` and the docs to the real owner.

A missing `GPG_PRIVATE_KEY` fails the job with an explicit `::error::`, so a tag
never publishes an unsigned repository silently.

## Key rotation

1. generate a new key in the same keyring, export **both** public keys over
   `packaging/apt/keyring/ferrent.gpg` (old + new, so old signatures still
   verify), update the fingerprint in this file and in `docs/install-debian.md`;
2. update the three secrets (`GPG_PRIVATE_KEY`, `GPG_KEY_ID`,
   `GPG_PASSPHRASE`), run the workflow on a tag;
3. clients must re-download the keyring (`/ferrent.gpg`) — `Signed-By` reads
   that file, so until then `apt update` fails with `NO_PUBKEY`.

`build-repo.sh` signs with a single key (`GPG_KEY_ID`); the old key is kept in
the public keyring only for verification during the transition. Remove it once
the clients have refreshed.
