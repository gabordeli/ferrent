#!/usr/bin/env bash
# Build the ferrentd .deb with a statically linked libtorrent inside a
# container, so the package runs on any Debian/Ubuntu with the glibc and
# libstdc++ baseline only (no libtorrent-rasterbar package).
#
#   packaging/debian/build.sh [suite] [arch]
#
#   suite  base image (default debian:12) — the .deb installs here and newer
#   arch   amd64 (default) or arm64 (cross toolchain, no emulation needed)
#
# The build is reproducible: the same suite, arch and LT_VERSION produce the
# same dependency set and library versions. Prints the .deb path on success and
# copies it to target/debian-dist/.
set -euo pipefail

suite=${1:-debian:12}
arch=${2:-amd64}
LT_VERSION=${LT_VERSION:-2.0.11}
CARGO_DEB_VERSION=${CARGO_DEB_VERSION:-3.8.0}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case "$suite" in
    ubuntu:*) base=$suite ;; # Ubuntu has no -slim tags
    *) base="$suite-slim" ;;
esac
tag="ferrent-deb-builder:${suite/:/}-${arch}"
out="$root/target/debian-dist"

# Cargo.toml is the version source of truth; a release tag must match it and
# only changes how the Debian version is rendered (0.1.0-rc1 -> 0.1.0~rc1-1).
if [ -n "${TAG:-}" ]; then
    deb_version=$(DEB_REVISION="${DEB_REVISION:-1}" "$root/packaging/debian/version.sh" "$TAG")
else
    deb_version=$(DEB_REVISION="${DEB_REVISION:-1}" "$root/packaging/debian/version.sh")
fi
echo "== version $deb_version${TAG:+ (tag $TAG)}"

case "$arch" in
amd64)
    arch_apt=""
    arch_pre=""
    lt_toolchain=""
    rust_target=""
    rust_target_setup=""
    link_env=""
    cargo_target=""
    pkg_config_path="/opt/lt/lib/pkgconfig"
    # The GUI is amd64-only for now: cross-building GTK for arm64 needs the
    # whole -dev:arm64 pkg-config set, and the server side does not run it.
    gui=1
    ;;
arm64)
    arch_pre="RUN dpkg --add-architecture arm64"
    arch_apt="g++-aarch64-linux-gnu gcc-aarch64-linux-gnu libssl-dev:arm64 zlib1g-dev:arm64"
    lt_toolchain="-DCMAKE_TOOLCHAIN_FILE=/tmp/toolchain-aarch64.cmake"
    rust_target="aarch64-unknown-linux-gnu"
    rust_target_setup="RUN rustup target add aarch64-unknown-linux-gnu"
    link_env="ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \\
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \\
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \\
    PKG_CONFIG_ALLOW_CROSS=1"
    cargo_target="--target aarch64-unknown-linux-gnu"
    pkg_config_path="/opt/lt/lib/pkgconfig:/usr/lib/aarch64-linux-gnu/pkgconfig"
    gui=0
    ;;
*)
    echo "unsupported arch: $arch (amd64|arm64)" >&2
    exit 1
    ;;
esac

gui_apt=""
gui_run=""
if [ "$gui" = 1 ]; then
    gui_apt="libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libglib2.0-dev libgtk-3-dev"
    gui_run='RUN cargo build --release -p ferrent-gui $cargo_target \
 && cargo deb -p ferrent-gui --no-build $cargo_target \
      --deb-version "$DEB_VERSION" --output /out'
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cat > "$work/Dockerfile" <<EOF
FROM $base
ARG DEB_VERSION
ENV DEBIAN_FRONTEND=noninteractive
$arch_pre
RUN apt-get update \\
 && apt-get install -y --no-install-recommends \\
      build-essential cmake pkg-config libboost-dev libssl-dev zlib1g-dev \\
      curl ca-certificates file $arch_apt $gui_apt \\
 && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \\
 && . /root/.cargo/env && cargo install cargo-deb --version $CARGO_DEB_VERSION --locked
ENV PATH=/root/.cargo/bin:\$PATH
$rust_target_setup
COPY packaging/debian/toolchain-aarch64.cmake /tmp/toolchain-aarch64.cmake

# Static libtorrent: its pkg-config file then carries ssl and zlib only, so
# the daemon has no libtorrent runtime dependency.
RUN curl -sSL -o /tmp/lt.tar.gz \\
      https://github.com/arvidn/libtorrent/releases/download/v$LT_VERSION/libtorrent-rasterbar-$LT_VERSION.tar.gz \\
 && tar -xzf /tmp/lt.tar.gz -C /tmp \\
 && cmake -S /tmp/libtorrent-rasterbar-$LT_VERSION -B /tmp/lt-build \\
      -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \\
      -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DCMAKE_INSTALL_PREFIX=/opt/lt \\
      $lt_toolchain -DBoost_INCLUDE_DIR=/usr/include \\
      -Dbuild_tests=OFF -Dbuild_examples=OFF -Dbuild_tools=OFF -Dpython-bindings=OFF \\
 && cmake --build /tmp/lt-build -j"\$(nproc)" \\
 && cmake --install /tmp/lt-build \\
 && rm -rf /tmp/lt-build /tmp/libtorrent-rasterbar-$LT_VERSION /tmp/lt.tar.gz

COPY . /src
WORKDIR /src
ENV PKG_CONFIG_PATH=$pkg_config_path FERRENT_LIBTORRENT_STATIC=1
$link_env
RUN cargo build --release -p ferrent-daemon --features libtorrent $cargo_target \\
 && cargo build --release -p ferrent-cli $cargo_target \\
 && cargo deb -p ferrent-daemon --features libtorrent --no-build $cargo_target \\
      --deb-version "\$DEB_VERSION" --output /out
$gui_run
RUN dpkg-deb -f /out/*.deb Package Version Architecture
EOF

# Handy when the generated recipe has to be checked without a 10 minute build.
if [ -n "${FERRENT_DEB_PRINT_DOCKERFILE:-}" ]; then
    cat "$work/Dockerfile"
    exit 0
fi

echo "== building $tag (libtorrent $LT_VERSION, static)"
# Plain progress: a failing RUN must leave the compiler output in the CI log.
docker build --progress=plain -f "$work/Dockerfile" --build-arg DEB_VERSION="$deb_version" \
    -t "$tag" "$root"

cid=$(docker create "$tag")
mkdir -p "$out"
docker cp "$cid:/out/." "$out" >/dev/null
docker rm "$cid" >/dev/null
echo "== $(ls -l "$out"/*.deb | awk '{print $5, $9}')"
