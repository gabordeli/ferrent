# Telepítés Windows alá

**Állapot: kész (2026-11-23), Windows-on még nem futtatva.** A binárisok
Linuxról, mingw-vel cross-buildelnek; a futás-teszt (VirtualBox VM) még hátra
van, ezért az alábbi lépések a build útját már adják, a viselkedést nem
garantálják minden ponton.

## Telepítés ZIP-ből

A `windows-cross` CI job artifactja
`Ferrent-<verzió>-windows-x86_64.zip`, ezekkel:

| Fájl | Szerep |
|---|---|
| `ferrentd.exe` | daemon (libtorrent statikusan benne) |
| `ferrent-cli.exe` | parancssori kliens |
| `ferrent-gui.exe` | asztali kliens (iced) |
| `libstdc++-6.dll` | a daemon C++ runtimeja |
| `nssm.exe` + `service.ps1` | Windows service wrapper |
| `config.example.toml` | minta konfiguráció |

Csomagold ki egy könyvtárba, majd **emelt szintű PowerShellben**:

```powershell
powershell -ExecutionPolicy Bypass -File .\service.ps1 install
```

Ez létrehozza a `%ProgramData%\ferrent` könyvtárat (data, token, logs,
`config.toml`), regisztrálja a `Ferrent` service-t és elindítja. A token a
kimenetben és a `%ProgramData%\ferrent\token` fájlban van; ezt kéri a GUI a
Servers modalban (`https://127.0.0.1:50051`).

```powershell
.\service.ps1 status | stop | start | restart | uninstall
```

A naplók: `%ProgramData%\ferrent\logs\ferrentd.{out,err}.log`.

## Miért NSSM

A `ferrentd` konzolprogram, és **Ctrl-C-re** menti a fastresume-ot. Az NSSM a
service stopnál `AppStopMethodConsole`-lal konzol-eseményt küld a
gyereknek, tehát a leállítás ugyanazt a mentést adja, mint a Ctrl-C; a
`sc stop` `TerminateProcess`-szel kilőné a folyamatot.

## Kézi futtatás (service nélkül)

```powershell
set FERRENT_CONFIG=C:\ProgramData\ferrent\config.toml
.\ferrentd.exe
```

## Build forrásból (Linuxról cross-compile)

Windows-on nem kell MSVC: a `x86_64-pc-windows-gnu` target mingw-vel linkel.
A recept és a döntések: [packaging/windows/README.md](../packaging/windows/README.md).

```bash
packaging/windows/build-libtorrent.sh /tmp/lt-win/prefix
PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=/tmp/lt-win/prefix/lib/pkgconfig \
  FERRENT_LIBTORRENT_STATIC=1 RUSTFLAGS="-C link-args=-static" \
  cargo build --release --target x86_64-pc-windows-gnu \
    -p ferrent-daemon -p ferrent-cli -p ferrent-gui
packaging/windows/stage-artifacts.sh && packaging/windows/make-zip.sh
```

## Ismert hiányosságok

- A GUI konfigja még a `%APPDATA%` helyett a Linuxos `~/.config` logikát
  követi; a daemon alapértékei (`/etc/ferrent/...`) helyett a service-generált
  config a mérvadó.
- Nincs `ferrent.ico`, így a ZIP-ben az exék alapértelmezett ikonnal
  jelennek meg, és nincs MSI/winget csomag.
- A 11 `#![cfg(unix)]` integrációs teszt Windows-on nem fut; a lib-tesztek
  igen.

A kutatás (opciók, mérések): [docs/research/windows.md](research/windows.md).
