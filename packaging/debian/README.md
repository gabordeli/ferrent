# ferrentd Debian package

Build (with the real engine):

```bash
cargo deb -p ferrent-daemon --features libtorrent
```

The dependency list is resolved at build time (`$auto`), so the package installs
only on the release it was built on: a trixie build needs `libc6 >= 2.39` and
`libtorrent-rasterbar2.0t64`, while bookworm has `libc6 2.36` and
`libtorrent-rasterbar2.0`. Build on the oldest suite you support:

```bash
mkdir -p /tmp/ferrent-deb-build
docker run --rm -v "$PWD:/src:ro" -v /tmp/ferrent-deb-build:/build debian:12 \
  bash -c 'set -e; apt-get update -qq &&
    apt-get install -y -qq build-essential pkg-config libssl-dev \
        libtorrent-rasterbar-dev curl ca-certificates >/dev/null &&
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null &&
    export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/build/target &&
    cargo install cargo-deb --version 3.8.0 --locked &&
    cd /src && cargo build --release -p ferrent-daemon --features libtorrent &&
    cargo deb -p ferrent-daemon --features libtorrent --no-build --output /build'
```

Verify a built package in a throwaway systemd container (31 asserts: user, file
modes, no secret in the package, start/stop, remove/purge):

```bash
packaging/debian/container-test.sh debian:12 target/debian/ferrentd_0.1.0-1_amd64.deb
```

Results and the container pitfalls: `docs/measurements/packaging.md`.

The package installs:

- `/usr/bin/ferrentd`
- `/lib/systemd/system/ferrentd.service`
- `/etc/ferrent/config.toml` (conffile, `0640 root:ferrent`)
- `/usr/share/man/man1/ferrentd.1.gz`

`postinst` creates the `ferrent` system user/group, `/var/lib/ferrent`, and a
random API token in `/etc/ferrent/token` (`0640 root:ferrent`). The token is
never part of the package contents.

`apt remove ferrentd` stops the service and keeps `/etc/ferrent` and
`/var/lib/ferrent`; `apt purge ferrentd` removes `/var/lib/ferrent`, the
generated token and the conffile. The `ferrent` user survives a purge (it may
own data the admin keeps), so reinstall keeps the same UID.

The unit uses `FERRENT_CONFIG=/etc/ferrent/config.toml`, waits up to 120 s for
a clean shutdown (fastresume save), and limits the service to
`/var/lib/ferrent` writes under `ProtectSystem=strict`.
