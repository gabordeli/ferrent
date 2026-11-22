#!/usr/bin/env bash
# Builds the AppImage in a container, so the runner needs neither Rust nor the
# GTK development packages (the shell executor only has docker).
#
#   packaging/appimage/container-build.sh [suite]
#
#   suite  base image (default debian:12) — the oldest baseline we support
#
# The AppImage lands in dist/ on the host. Prints the image path on success.
set -euo pipefail

suite=${1:-debian:12}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
case "$suite" in
    ubuntu:*) base=$suite ;;
    *) base="$suite-slim" ;;
esac
tag="ferrent-appimage-builder:${suite/:/}"
out="$root/dist"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# libfontconfig1/fribidi/wayland/xkbcommon are the runtime libraries build.sh
# pulls in explicitly: a minimal container does not ship them, a desktop does.
cat > "$work/Dockerfile" <<EOF
FROM $base
ENV DEBIAN_FRONTEND=noninteractive APPIMAGE_EXTRACT_AND_RUN=1
RUN apt-get update \\
 && apt-get install -y --no-install-recommends \\
      build-essential pkg-config ca-certificates curl file libssl-dev \\
      libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev \\
      libxrandr-dev libxi-dev libglib2.0-dev libgtk-3-dev \\
      libfontconfig1 libfribidi0 libwayland-client0 libwayland-cursor0 \\
      libwayland-egl1 libxkbcommon0 libxkbcommon-x11-0 \\
      libgdk-pixbuf-2.0-0 librsvg2-2 \\
 && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH=/root/.cargo/bin:\$PATH
COPY . /src
WORKDIR /src
RUN packaging/appimage/build.sh
EOF

if [ -n "${FERRENT_APPIMAGE_PRINT_DOCKERFILE:-}" ]; then
    cat "$work/Dockerfile"
    exit 0
fi

echo "== building $tag"
# Dropping the previous image keeps the runner's disk free; the AppImage build
# is release-only, so the image is disposable.
docker image rm -f "$tag" >/dev/null 2>&1 || true
docker build --progress=plain -f "$work/Dockerfile" -t "$tag" "$root"

cid=$(docker create "$tag")
mkdir -p "$out"
docker cp "$cid:/src/dist/." "$out" >/dev/null
docker rm "$cid" >/dev/null
docker image rm -f "$tag" >/dev/null 2>&1 || true
rm -rf "$out/Ferrent.AppDir"
echo "== $(ls -l "$out"/Ferrent-*.AppImage | awk '{print $5, $9}')"
