# ADR 0005 – Minimum supported Debian and Ubuntu

**Status:** accepted (2026-11-20)
**Context:** M3-T2 (`.deb` for several distros, static libtorrent)

## Context

`cargo deb` resolves `Depends` from the linked libraries (`dpkg-shlibdeps`). With
a dynamically linked libtorrent the package is tied to the build host's ABI:

| Built on | Resulting Depends |
|---|---|
| Debian 12 (bookworm) | `libc6 (>= 2.34), libssl3, libtorrent-rasterbar2.0 (>= 2.0.8)` |
| Debian 13 (trixie) | `libc6 (>= 2.39), libssl3t64, libtorrent-rasterbar2.0t64 (>= 2.0.11)` |

A trixie build therefore does not install on bookworm (glibc 2.36, no `t64`
libs), and either build pins one libtorrent ABI version. libtorrent's C++ ABI
also means a distro upgrade of the shared library can break the daemon.

## Decision

1. **Link libtorrent statically.** The build container compiles libtorrent
   2.0.11 from source with `BUILD_SHARED_LIBS=OFF` and links the daemon with
   `FERRENT_LIBTORRENT_STATIC=1` (pkg-config `--static`). OpenSSL and zlib stay
   dynamic, so security updates reach the daemon.
2. **Build on the oldest supported suite** (`debian:12`), because the glibc and
   libstdc++ symbols come from the build host.
3. **Support Debian 12, Debian 13 and Ubuntu 24.04** on amd64 and arm64. The
   same `.deb` installs on all of them; it is verified by
   `packaging/debian/container-test.sh` (32 asserts, incl. "no libtorrent
   runtime dependency").
4. Ubuntu 22.04 is not supported: it was not tested, and the glibc/libstdc++
   combination is only one step away from the baseline.

## Consequences

- `Depends` shrinks to `libc6 (>= 2.34), libssl3 (>= 3.0.0), libstdc++6 (>= 12)`
  plus `adduser` and `init-system-helpers`.
- The binary grows from 15,2 MB to 26,8 MB (amd64), the `.deb` from 3,79 MB to
  5,27 MB; installed size 19,5 MB. Details: `docs/measurements/packaging.md`.
- The build takes ~8 minutes and needs a container with cmake, a C++ toolchain
  and the Rust toolchain; `packaging/debian/build.sh` encapsulates it.
- arm64 is cross-compiled with the Debian `aarch64-linux-gnu` toolchain (no
  emulation); the smoke run under `qemu-aarch64-static` starts, opens TLS and
  persists state.
- Upgrading libtorrent is now a deliberate change in `build.sh` (`LT_VERSION`),
  not whatever the host ships.
