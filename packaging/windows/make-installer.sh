#!/usr/bin/env bash
# Builds the NSIS installer from the staged binaries.
#
# Usage: make-installer.sh [staged-dir] [out-dir]
#   staged-dir defaults to target/windows (stage-artifacts.sh output)
#   out-dir    defaults to dist
#
# The installer ships the same file set as the ZIP and calls the same
# service.ps1, so the two routes cannot drift apart.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
staged=${1:-$root/target/windows}
out_dir=${2:-$root/dist}
version=$(bash "$root/packaging/debian/version.sh" --cargo)
name=Ferrent-$version-windows-x86_64

for file in ferrentd.exe ferrent-cli.exe ferrent-gui.exe libstdc++-6.dll; do
  [ -f "$staged/$file" ] || { echo "missing $staged/$file (run stage-artifacts.sh)" >&2; exit 1; }
done

command -v makensis >/dev/null || { echo "makensis not found (apt install nsis)" >&2; exit 1; }

# NSIS wants four numeric parts for the version resource; the -rc suffix only
# goes into the display strings.
version4=$(printf '%s' "${version%%-*}" |
  awk -F. '{ printf "%d.%d.%d.%d", $1, $2, $3, $4 }')

build=$out_dir/$name-installer
rm -rf "$build"
mkdir -p "$build"
cp "$staged/ferrentd.exe" "$staged/ferrent-cli.exe" "$staged/ferrent-gui.exe" \
  "$staged/libstdc++-6.dll" "$build"/
cp "$root/packaging/windows/config.example.toml" "$root/packaging/windows/service.ps1" \
  "$root/packaging/windows/README.txt" "$root/packaging/windows/ferrent.ico" "$build"/

nssm=${NSSM_EXE:-$root/packaging/windows/vendor/nssm-2.24-x64.exe}
[ -f "$nssm" ] || { echo "missing $nssm (see packaging/windows/vendor/README.md)" >&2; exit 1; }
cp "$nssm" "$build/nssm.exe"

makensis -V2 \
  -DSRCDIR="$build" \
  -DOUTFILE="$out_dir/$name-setup.exe" \
  -DVERSION="$version" \
  -DVERSION4="$version4" \
  -DICONFILE="$root/packaging/windows/ferrent.ico" \
  -DREPO_ROOT="$root" \
  "$root/packaging/windows/installer.nsi"

rm -rf "$build"
echo "$out_dir/$name-setup.exe"
