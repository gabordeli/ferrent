#!/usr/bin/env bash
# Builds dist/Ferrent-<version>-<arch>.AppImage: the GUI with GTK and its
# loaders bundled.
#
#   packaging/appimage/build.sh [--no-build]
#
#   --no-build   reuse target/release/ferrent-gui instead of running cargo
#
# Downloads linuxdeploy, its GTK plugin and appimagetool into .tools/appimage
# on first use and verifies their sha256 against packaging/appimage/tools.lock,
# so a changed upstream artifact fails instead of silently shipping. Needs FUSE,
# or APPIMAGE_EXTRACT_AND_RUN=1, which the script sets for the tools.
set -euo pipefail

cd "$(dirname "$0")/../.."

build=1
for arg in "$@"; do
    case "$arg" in
    --no-build) build=0 ;;
    *) echo "usage: $0 [--no-build]" >&2; exit 2 ;;
    esac
done

arch=${ARCH:-x86_64}
tools=${TOOLS:-.tools/appimage}
dist=${DIST:-dist}
version=${VERSION:-$(packaging/debian/version.sh --cargo)}
lock=packaging/appimage/tools.lock
out="$dist/Ferrent-$version-$arch.AppImage"

case "$arch" in
x86_64) arch_libdir=x86_64-linux-gnu ;;
*)
    # The AppImage is x86_64 only: the arm64 target is covered by the .deb and
    # the runner is amd64 (an arm64 bundling would need qemu or a native arm64
    # builder).
    echo "unsupported arch: $arch (x86_64)" >&2
    exit 1
    ;;
esac

mkdir -p "$tools" "$dist"

fetch() {
    local name=$1 url=$2 path="$tools/$1"
    if [ ! -x "$path" ]; then
        echo "fetching $name"
        curl -fsSL -o "$path" "$url"
        chmod +x "$path"
    fi
    local expected got
    expected=$(awk -v name="$name" '$2 == name { print $1 }' "$lock")
    got=$(sha256sum "$path" | cut -d' ' -f1)
    if [ -z "$expected" ]; then
        echo "$name is not in $lock (expected "$got")" >&2
        exit 1
    fi
    if [ "$expected" != "$got" ]; then
        echo "$name sha256 mismatch: lock $expected, file $got" >&2
        echo "delete $path and update $lock if the new upstream build is wanted" >&2
        exit 1
    fi
}

export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH="$arch"
fetch linuxdeploy \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch}.AppImage"
fetch appimagetool \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${arch}.AppImage"
fetch linuxdeploy-plugin-gtk.sh \
    "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh"

if [ "$build" = 1 ]; then
    cargo_bin=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}
    "$cargo_bin" build --release -p ferrent-gui
fi
[ -x target/release/ferrent-gui ] || {
    echo "target/release/ferrent-gui is missing; run without --no-build" >&2
    exit 1
}

appdir=$dist/Ferrent.AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m 0644 packaging/appimage/ferrent.desktop "$appdir/usr/share/applications/"
install -m 0644 crates/gui/assets/mark.png \
    "$appdir/usr/share/icons/hicolor/256x256/apps/ferrent.png"

# linuxdeploy skips a few libraries a desktop distro always has; a minimal
# container does not, so they are pulled in explicitly. The X client libs are
# here for the headless smoke (Xvfb + the AppImage in a bare debian:12).
extra=(
    libfontconfig.so.1
    libfribidi.so.0
    libwayland-client.so.0
    libwayland-cursor.so.0
    libwayland-egl.so.1
    libxkbcommon.so.0
    libxkbcommon-x11.so.0
    libexpat.so.1
    libfreetype.so.6
    libharfbuzz.so.0
    libX11.so.6
    libxcb.so.1
)
libs=()
for lib in "${extra[@]}"; do
    libs+=(--library "/usr/lib/$arch_libdir/$lib")
done

PATH="$tools:$PATH" "$tools/linuxdeploy" \
    --appdir "$appdir" \
    --executable target/release/ferrent-gui \
    --desktop-file packaging/appimage/ferrent.desktop \
    --icon-file packaging/appimage/ferrent.png \
    "${libs[@]}" \
    --plugin gtk

rm -f "$out"
"$tools/appimagetool" "$appdir" "$out"
echo "built $out ($(du -h "$out" | cut -f1))"
