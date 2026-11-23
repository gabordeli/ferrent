#!/usr/bin/env bash
# Stages the Windows build output: stripped binaries plus the mingw C++ runtime
# the daemon needs at run time.
#
# The .deb keeps its symbols because the SDK resolves panic frames in-process;
# here the trade-off is the other way: 124 MB per unstripped binary is too much
# for a ZIP, so the artifacts are stripped (the first runtime pass on the VM
# decides whether a symbol story is needed).
set -euo pipefail

target=${1:-target/x86_64-pc-windows-gnu/release}
out=${2:-target/windows}

mkdir -p "$out"
strip_tool=${STRIP:-x86_64-w64-mingw32-strip}

for name in ferrentd ferrent-cli ferrent-gui; do
  "$strip_tool" --strip-all -o "$out/$name.exe" "$target/$name.exe"
done

# cxx links the C++ runtime dynamically even with `-static` (rustc wraps native
# libs in -Bdynamic), so libstdc++-6.dll travels next to the daemon.
dll=$(x86_64-w64-mingw32-g++ -print-file-name=libstdc++-6.dll)
"$strip_tool" --strip-all -o "$out/libstdc++-6.dll" "$dll"

ls -l "$out"

# A gate, not a report: the daemon must carry libtorrent statically. A dynamic
# dependency would only show up as a missing DLL on the Windows box, so any
# import outside the Windows API and the C++ runtime fails the build here.
allow='^(kernel32|ntdll|msvcrt|advapi32|user32|api-ms-win[a-z0-9._-]*|bcrypt|bcryptprimitives|crypt32|iphlpapi|mswsock|ws2_32|secur32|userenv|ncrypt|libstdc\+\+-6)\.dll$'
for exe in "$out"/*.exe; do
  unexpected=$(x86_64-w64-mingw32-objdump -p "$exe" |
    awk '/DLL Name/{print $3}' | sort -u | grep -viE "$allow" || true)
  if [ -n "$unexpected" ]; then
    echo "$exe imports unexpected libraries:" >&2
    echo "$unexpected" >&2
    exit 1
  fi
done
