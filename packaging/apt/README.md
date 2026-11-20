# Ferrent APT repository

The `.deb` files are published from the `gh-pages` branch as a signed APT
repository (`dists/stable` + `pool/main/f/ferrentd`). The client side is
described in `docs/apt-install.md`.

## Signing key

| | |
|---|---|
| UID | `Ferrent APT Repository <apt@ferrent.invalid>` |
| Fingerprint | `DB161AB5A22473586AC52CEDD727691B9D3B58AA` |
| Type | RSA 4096, sign only |
| Valid until | 2031-11-19 |

The public key is tracked as `packaging/apt/keyring/ferrent.gpg` and is also
served next to the repository (`/ferrent.gpg`), so `apt` can fetch it over
HTTPS.

The private key lives **outside the repository**, in a dedicated keyring so it
does not mix with the personal one:

```bash
export GNUPGHOME="$HOME/.config/ferrent/apt/gnupg"
# armored secret key, mode 0600:
#   $GNUPGHOME/../ferrent-apt-private.asc
```

GitHub secrets (repository settings → Secrets and variables → Actions):

| Secret | Value |
|---|---|
| `GPG_PRIVATE_KEY` | `base64 -w0 ~/.config/ferrent/apt/ferrent-apt-private.asc` |
| `GPG_KEY_ID` | `DB161AB5A22473586AC52CEDD727691B9D3B58AA` |

## Build and test locally

```bash
export GNUPGHOME="$HOME/.config/ferrent/apt/gnupg"
export GPG_KEY_ID=DB161AB5A22473586AC52CEDD727691B9D3B58AA

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

`.github/workflows/release-apt.yml` runs on a `v*` tag:

1. builds the two `.deb` files in the container,
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

1. generate a new key in the same keyring, export it over
   `packaging/apt/keyring/ferrent.gpg`, update the fingerprint in this file and
   in `docs/apt-install.md`;
2. update the two secrets, run the workflow on a tag;
3. clients must re-download the keyring (`/ferrent.gpg`) — `Signed-By` reads that
   file, so until then `apt update` fails with `NO_PUBKEY`.

Keep the old key in the keyring during the transition if both signatures are
wanted; the current `build-repo.sh` signs with a single key (`GPG_KEY_ID`).
