# Telepítés Windows alá

**Állapot: kész (2026-11-23), Windows-on még nem futtatva.** A binárisok
Linuxról, mingw-vel cross-buildelnek; a futás-teszt (VirtualBox VM) még hátra
van, ezért az alábbi lépések a build útját már adják, a viselkedést nem
garantálják minden ponton.

## Telepítés a setup.exe-vel (ajánlott)

A `windows-cross` CI job artifactja
`Ferrent-<verzió>-windows-x86_64-setup.exe`. Futtatása (dupla kattintás,
esetleg **jobb klikk → Run as administrator**):

1. varázsló: licence, összetevők, célkönyvtár (`C:\Program Files\Ferrent`);
2. kimásolja a binárisokat, a DLL-t, az `nssm.exe`-t és a `service.ps1`-t;
3. beregisztrálja és elindítja a **`Ferrent` service**-t (automatikus indítás);
4. a végén kiírja a **tokent** (és a `%ProgramData%\ferrent\token` fájlra
   mutat) — ezt kéri a GUI a Servers modalban, a
   `https://127.0.0.1:50051` címhez.

A Start-menüben `Ferrent` (GUI) és `Uninstall` lesz, opcionálisan asztali
parancsikon. Az uninstaller előbb leállítja a service-t, majd megkérdezi,
hogy a `%ProgramData%\ferrent` adatkönyvtárat is töröljük-e — **alapból
meghagyja** (a torrentek és az állapot ott vannak).

A SmartScreen ismeretlen kiadót jelez (a setup.exe nincs code-signolva):
"More info → Run anyway".

## Telepítés ZIP-ből (szerver, kézi út)

Ugyanaz a fájlkészlet, service nélküli varázsló nélkül. A ZIP tartalma:

| Fájl | Szerep |
|---|---|
| `ferrentd.exe` | daemon (libtorrent statikusan benne) |
| `ferrent-cli.exe` | parancssori kliens |
| `ferrent-gui.exe` | asztali kliens (iced) |
| `libstdc++-6.dll` | a daemon C++ runtimeja |
| `nssm.exe` + `service.ps1` | Windows service wrapper |
| `config.example.toml` | minta konfiguráció |

A ZIP-ben minden egy mappában van, DLL-t nem kell máshova másolni. Csomagold
ki egy könyvtárba, majd **emelt szintű PowerShellben**:

```powershell
powershell -ExecutionPolicy Bypass -File .\service.ps1 install
```

Ez létrehozza a `%ProgramData%\ferrent` könyvtárat (data, token, logs,
`config.toml`), regisztrálja a `Ferrent` service-t és elindítja. A token a
kimenetben és a `%ProgramData%\ferrent\token` fájlban van.

```powershell
.\service.ps1 status | stop | start | restart | uninstall
```

A naplók: `%ProgramData%\ferrent\logs\ferrentd.{out,err}.log`.

A `token` és a `config.toml` a `%ProgramData%` örökölt ACL-je helyett
**SYSTEM + Administrators** (teljes), öröklés nélkül — a Linux-os
`0640 root:ferrent` megfelelője; a daemon induláskor a tokenre ugyanezt újra
érvényesíti.

## Miért NSSM

A `ferrentd` konzolprogram, és **Ctrl-C-re** menti a fastresume-ot. Az NSSM a
service stopnál `AppStopMethodConsole`-lal konzol-eseményt küld a
gyereknek, tehát a leállítás ugyanazt a mentést adja, mint a Ctrl-C; a
`sc stop` `TerminateProcess`-szel kilőné a folyamatot. A setup.exe és a ZIP
ugyanezt a `service.ps1`-t hívja, így a két út nem tud szétcsúszni.

## Kézi futtatás (service nélkül)

A daemon a Windows-os alapértékeket használja: config
`%ProgramData%\ferrent\config.toml`, token `%ProgramData%\ferrent\token`,
adat `%ProgramData%\ferrent\data` — ugyanazok, amiket a `service.ps1 install`
létrehoz. Env felülírás: `FERRENT_CONFIG`, `FERRENT_DATA_DIR`,
`FERRENT_TOKEN`.

```powershell
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
packaging/windows/stage-artifacts.sh
packaging/windows/make-zip.sh
packaging/windows/make-installer.sh
```

## Ismert hiányosságok

- A GUI konfigja még a `%APPDATA%` helyett a Linuxos `~/.config` logikát
  követi; a daemon viszont már a `%ProgramData%\ferrent` alapértékeket
  használja.
- **Az Import oldal rejtett a GUI-ban**: a daemon `GetStatus`-ban
  `import_enabled=false`-t ad Windows-on, mert a Deluge Windows-os állapotkönyvtára,
  plugin-configjai és a `D:\...` mentési utak még nem lettek tesztelve. A
  `ferrent-cli import-deluge` változatlanul működik (ez a VM-es teszt eszköze);
  a futás után a `platform::import_supported()` egy sora billenti át.
- Nincs MSI és winget csomag; a setup.exe nincs code-signolva (ezért a
  SmartScreen ismeretlen kiadót jelez).
- A 11 `#![cfg(unix)]` integrációs teszt Windows-on nem fut; a lib-tesztek
  igen.

A kutatás (opciók, mérések): [docs/research/windows.md](research/windows.md).
