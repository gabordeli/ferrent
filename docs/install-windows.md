# Telepítés Windows alá

**Állapot: tervezett.** Windows build még nem készült (a kutatás a #77
issue-ban folyik); az alábbi lépések a várható utat írják le.

## Ami kell hozzá

- MSVC toolchain (Visual Studio Build Tools, „Desktop development with C++”).
- Rust stable (`rustup`), `x86_64-pc-windows-msvc`.
- libtorrent 2.0 és boost: `vcpkg install libtorrent:x64-windows boost:x64-windows`.
- A GUI (iced/winit) Windowson a natív backendet használja.

## Várható build

```powershell
vcpkg install libtorrent:x64-windows boost:x64-windows
$env:VCPKG_ROOT = "C:\vcpkg"
cargo build --release -p ferrent-daemon --features libtorrent
cargo build --release -p ferrent-cli -p ferrent-gui
```

## Nyitott pontok

- A `cxx` bridge MSVC-vel fordul-e (a libtorrent 2.0 C++17 API-ja), és a
  statikus linkelés hogyan megy `.exe`-be.
- A TLS cert + fingerprint pinning platformfüggetlen, de a
  `ferrent.local` névhez hosts bejegyzés kell.
- A daemon szolgáltatásként (Windows Service) futtatása még nincs meg.
- Az installer (MSI/MSIX) és az auto-update nem része a tervnek.

Ha elkészül, ez a doksi kapja a működő lépéseket; addig a Linux az
ajánlott platform ([docs/install-debian.md](install-debian.md)).
