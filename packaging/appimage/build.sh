#!/usr/bin/env bash
# Builds dist/Ferrent-<arch>.AppImage: the GUI with GTK and its loaders bundled.
#
#   packaging/appimage/build.sh
#
# Downloads linuxdeploy, its GTK plugin and appimagetool into .tools/appimage
# on first use (the tools themselves are not committed). Needs FUSE, or
# APPIMAGE_EXTRACT_AND_RUN=1, which the script sets for the tools.
set -euo pipefail

cd "$(dirname "$0")/../.."

arch=${ARCH:-x86_64}
tools=${TOOLS:-.tools/appimage}
dist=dist
mkdir -p "$tools" "$dist"

fetch() {
    local path=$1 url=$2
    if [ ! -x "$path" ]; then
        echo "fetching $(basename "$path")"
        curl -fsSL -o "$path" "$url"
        chmod +x "$path"
    fi
}

export APPIMAGE_EXTRACT_AND_RUN=1
export ARCH="$arch"
fetch "$tools/linuxdeploy" \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch}.AppImage"
fetch "$tools/appimagetool" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${arch}.AppImage"
fetch "$tools/linuxdeploy-plugin-gtk.sh" \
    "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh"

cargo_bin=${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}
"$cargo_bin" build --release -p ferrent-gui

appdir=$dist/Ferrent.AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m 0644 packaging/appimage/ferrent.desktop "$appdir/usr/share/applications/"
install -m 0644 crates/gui/assets/mark.png \
    "$appdir/usr/share/icons/hicolor/256x256/apps/ferrent.png"

# linuxdeploy skips a few libraries a desktop distro always has; a minimal
# container does not, so they are pulled in explicitly.
extra=(
    libfontconfig.so.1
    libfribidi.so.0
    libwayland-client.so.0
    libwayland-cursor.so.0
    libwayland-egl.so.1
    libxkbcommon.so.0
    libxkbcommon-x11.so.0
)
libs=()
for lib in "${extra[@]}"; do
    libs+=(--library "/usr/lib/x86_64-linux-gnu/$lib")
done

PATH="$tools:$PATH" "$tools/linuxdeploy" \
    --appdir "$appdir" \
    --executable target/release/ferrent-gui \
    --desktop-file packaging/appimage/ferrent.desktop \
    --icon-file packaging/appimage/ferrent.png \
    "${libs[@]}" \
    --plugin gtk

"$tools/appimagetool" "$appdir" "$dist/Ferrent-${arch}.AppImage"
echo "built $dist/Ferrent-${arch}.AppImage"
