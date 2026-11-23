# Vendored binaries

## `nssm-2.24-x64.exe`

NSSM (the Non-Sucking Service Manager) 2.24, the 64-bit build from the
official `nssm-2.24.zip` (nssm.cc). It wraps `ferrentd.exe` as a Windows
service and — the reason it is here — stops it with a **console event**, which
is what makes the daemon save fastresume; `sc stop` would terminate the
process instead.

- License: public domain (nssm.cc, "NSSM is public domain").
- Size: 324 KB, `sha256 = f689ee9af94b00e9e3f0bb072b34caaf207f32dcb4f5782fc9ca351df9a06c97`.
- Vendored because nssm.cc answers 503 often enough that a release build should
  not depend on it (measured 2026-11-23: the first download worked, the next
  one within the hour did not).

The source is available at <https://nssm.cc/download>; replace this file and
the checksum above together if a newer version is wanted.
