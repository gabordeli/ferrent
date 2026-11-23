#!/usr/bin/env bash
# Builds the Windows ZIP from the staged binaries.
#
# Usage: make-zip.sh [staged-dir] [out-dir]
#   staged-dir defaults to target/windows (stage-artifacts.sh output)
#   out-dir    defaults to dist
#
# NSSM is vendored (packaging/windows/vendor): Ferrent needs it to stop the
# daemon with a console event (README.txt inside the ZIP explains why).
set -euo pipefail

staged=${1:-target/windows}
out_dir=${2:-dist}
version=$(bash packaging/debian/version.sh --cargo)
name=Ferrent-$version-windows-x86_64
root=$out_dir/$name

for file in ferrentd.exe ferrent-cli.exe ferrent-gui.exe libstdc++-6.dll; do
  [ -f "$staged/$file" ] || { echo "missing $staged/$file (run stage-artifacts.sh)" >&2; exit 1; }
done

rm -rf "$root"
mkdir -p "$root"
cp "$staged"/ferrentd.exe "$staged"/ferrent-cli.exe "$staged"/ferrent-gui.exe \
  "$staged"/libstdc++-6.dll "$root"/
cp packaging/windows/config.example.toml packaging/windows/service.ps1 \
  packaging/windows/README.txt "$root"/

nssm=${NSSM_EXE:-packaging/windows/vendor/nssm-2.24-x64.exe}
[ -f "$nssm" ] || { echo "missing $nssm (see packaging/windows/vendor/README.md)" >&2; exit 1; }
cp "$nssm" "$root/nssm.exe"

(cd "$out_dir" && zip -q -r -9 "$name.zip" "$name")
echo "$out_dir/$name.zip"
