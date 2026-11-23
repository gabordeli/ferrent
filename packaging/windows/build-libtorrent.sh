#!/usr/bin/env bash
# Cross-builds libtorrent for the Windows (mingw) target and installs it into a
# prefix that pkg-config can read. Idempotent: an existing archive is reused,
# so a CI cache only pays the build once.
#
# No OpenSSL: with no OpenSSL/GnuTLS/libgcrypt found, libtorrent falls back to
# its built-in RC4/SHA1, which is enough for a BitTorrent-only build and avoids
# cross-compiling OpenSSL.
set -euo pipefail

prefix=${1:?usage: build-libtorrent.sh <prefix>}
version=${LT_VERSION:-2.0.11}
jobs=${JOBS:-$(nproc)}
src=${LT_SRC:-/tmp/libtorrent-rasterbar-$version}
build=${LT_BUILD:-/tmp/lt-build}

if [ -f "$prefix/lib/libtorrent-rasterbar.a" ]; then
  echo "libtorrent $version: $prefix/lib/libtorrent-rasterbar.a exists, skipping"
  exit 0
fi

if [ ! -d "$src" ]; then
  tarball=/tmp/libtorrent-rasterbar-$version.tar.gz
  curl -fsSL -o "$tarball" \
    "https://github.com/arvidn/libtorrent/releases/download/v$version/libtorrent-rasterbar-$version.tar.gz"
  tar -C /tmp -xf "$tarball"
fi

# Boost is header-only here. Pointing the compiler at /usr/include directly
# would shadow the mingw headers with glibc's (wchar.h → bits/libc-header-start.h),
# so only the boost/ directory is exposed on a separate include root.
boost_include=${BOOST_INCLUDE_DIR:-/tmp/boost-include}
mkdir -p "$boost_include"
[ -e "$boost_include/boost" ] || ln -sfn /usr/include/boost "$boost_include/boost"

cmake -S "$src" -B "$build" -G "Unix Makefiles" \
  -DCMAKE_TOOLCHAIN_FILE="$(cd "$(dirname "$0")" && pwd)/toolchain-mingw.cmake" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -Dbuild_tests=OFF -Dbuild_examples=OFF -Dbuild_tools=OFF -Dpython-bindings=OFF \
  -DCMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON -DCMAKE_DISABLE_FIND_PACKAGE_GnuTLS=ON \
  -DBoost_INCLUDE_DIR="$boost_include" \
  -DCMAKE_INSTALL_PREFIX="$prefix"
cmake --build "$build" -j"$jobs"
cmake --install "$build"

echo "libtorrent $version installed into $prefix"
