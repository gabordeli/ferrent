# Ferrent Debian packages

Two packages come out of one container build:

- **`ferrent`** — the daemon (`/usr/bin/ferrentd`) and the CLI
  (`/usr/bin/ferrent-cli`), the systemd unit, the config and both man pages.
- **`ferrent-gui`** (amd64 only) — the iced thin client, its desktop entry and
  icon; `Depends: ferrent`.

Build with a statically linked libtorrent (no libtorrent runtime dependency,
one package for Debian 12/13 and Ubuntu 24.04):

```bash
packaging/debian/build.sh debian:12 amd64   # -> target/debian-dist/*.deb
packaging/debian/build.sh debian:12 arm64   # cross-compiled, no emulation
```

The script builds inside a container: it compiles libtorrent from source with
`BUILD_SHARED_LIBS=OFF` (version via `LT_VERSION`, default 2.0.11), then builds
the daemon and the CLI with `FERRENT_LIBTORRENT_STATIC=1` so pkg-config
resolves the static archive. OpenSSL and zlib stay dynamic. The decision is ADR
0005 (`docs/adr/0005-min-debian.md`).

The GUI is built in the same container on amd64 (the GTK packages are installed
for it). The arm64 build is a cross-compile and skips the GUI: the server side
does not run it, and cross-building GTK needs the whole `-dev:arm64` pkg-config
set.

A plain `cargo deb -p ferrent-daemon --features libtorrent` on the host still
works, but then `$auto` resolves against the host: a trixie build needs
`libc6 >= 2.39` and `libtorrent-rasterbar2.0t64` and does not install on
bookworm.

Verify a built package in a throwaway systemd container (32 asserts: user, file
modes, no secret in the package, no libtorrent dependency, start/stop,
remove/purge):

```bash
packaging/debian/container-test.sh debian:12 target/debian-dist/ferrent_0.1.0-1_amd64.deb
packaging/debian/container-test.sh ubuntu:24.04 target/debian-dist/ferrent_0.1.0-1_amd64.deb
```

Results and the container pitfalls: `docs/measurements/packaging.md`.

The `ferrent` package installs:

- `/usr/bin/ferrentd` and `/usr/bin/ferrent-cli`
- `/lib/systemd/system/ferrentd.service`
- `/etc/ferrent/config.toml` (conffile, `0640 root:ferrent`)
- `/usr/share/man/man1/ferrentd.1.gz`, `ferrent-cli.1.gz`

`ferrent-gui` installs `/usr/bin/ferrent-gui`, the desktop entry and the icon,
and depends on `ferrent`.

`postinst` creates the `ferrent` system user/group, `/var/lib/ferrent`, and a
random API token in `/etc/ferrent/token` (`0640 root:ferrent`). The token is
never part of the package contents.

`apt remove ferrent` stops the service and keeps `/etc/ferrent` and
`/var/lib/ferrent`; `apt purge ferrent` removes `/var/lib/ferrent`, the
generated token and the conffile. The `ferrent` user survives a purge (it may
own data the admin keeps), so reinstall keeps the same UID.

The 0.1.x packages were named `ferrentd`; `ferrent` declares
`Replaces/Breaks/Provides: ferrentd (<= 0.1.2-1)`, so an upgrade from the old
name replaces it in place while the service and the paths stay the same.

The unit uses `FERRENT_CONFIG=/etc/ferrent/config.toml`, waits up to 120 s for
a clean shutdown (fastresume save), and limits the service to
`/var/lib/ferrent` writes under `ProtectSystem=strict`.
