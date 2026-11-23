# Windows (mingw cross-build)

A Windows csomag Linuxról készül, `x86_64-pc-windows-gnu` targettel. A
`windows-cross` CI job ugyanezt futtatja a runner konténerében; ez a leírás a
kézi reprodukáláshoz kell.

## Előfeltételek

```bash
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 cmake make pkg-config \
  libboost-dev curl zip nsis
rustup target add x86_64-pc-windows-gnu
```

## Build

```bash
packaging/windows/build-libtorrent.sh /tmp/lt-win/prefix      # ~2 perc, utána cache-elhető

PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_PATH=/tmp/lt-win/prefix/lib/pkgconfig \
FERRENT_LIBTORRENT_STATIC=1 \
RUSTFLAGS="-C link-args=-static" \
cargo build --release --target x86_64-pc-windows-gnu \
  -p ferrent-daemon -p ferrent-cli -p ferrent-gui

packaging/windows/stage-artifacts.sh      # → target/windows/ (strippelt binárisok + DLL)
packaging/windows/make-zip.sh             # → target/windows/dist/Ferrent-<ver>-windows-x86_64.zip
packaging/windows/make-installer.sh       # → target/windows/dist/Ferrent-<ver>-windows-x86_64-setup.exe
```

## A ZIP tartalma

`ferrentd.exe`, `ferrent-cli.exe`, `ferrent-gui.exe`, `libstdc++-6.dll`,
`nssm.exe`, `service.ps1`, `config.example.toml`, `README.txt`.

## A telepítő (NSIS)

Ugyanaz a fájlkészlet, egy `.exe`-ben: varázsló, licence-oldal, Start-menü és
asztali parancsikon (`ferrent.ico`), registry-uninstall, és a **service
regisztrálása ugyanazzal a `service.ps1`-gyel**, amit a ZIP is szállít — így a
két út nem tud szétcsúszni. A telepítés végén a token a kimenetben jelenik
meg, és a `%ProgramData%\ferrent\token` fájlban marad.

A `make-installer.sh` a staged binárisokat egy ideiglenes könyvtárba másolja a
`service.ps1`-gyel, az `nssm.exe`-vel és a `ferrent.ico`-val, majd a
`makensis`-t hívja. A verzió ugyanaz a `packaging/debian/version.sh --cargo`,
mint a ZIP-nél és a `.deb`-nél; az NSIS négyrészes verzió-erőforrása a
`-rc1` suffixet levágja (`1.0.0.0`), a megjelenített verzió viszont teljes.

Az uninstaller a service-t előbb leállítja és törli (különben a `nssm.exe` és a
`service.ps1` foglalt fájl lenne), utána kérdez rá, hogy a
`%ProgramData%\ferrent` adatkönyvtárat is töröljük-e — alapból **meghagyja**
(a torrentek és az állapot ott vannak).

## Service

A daemon **konzolprogram**, amely Ctrl-C-re menti a fastresume-ot. Egy Windows
service-nek viszont `StartServiceCtrlDispatcher` kellene; ezért a wrapper
**NSSM**, amely `AppStopMethodConsole`-lal pontosan konzol-eseményt küld a
gyereknek — így a service stop ugyanazt a mentést adja, mint a Ctrl-C (a
`sc stop` `TerminateProcess`-szel kilőné).

```powershell
powershell -ExecutionPolicy Bypass -File .\service.ps1 install
```

Ez létrehozza a `%ProgramData%\ferrent` könyvtárat (data, token, logs,
config.toml), regisztrálja a `Ferrent` service-t és elindítja. A token a
kimenetben és a `%ProgramData%\ferrent\token` fájlban.

A `service.ps1` a configot **generálja** (nem másolja), hogy a `data_dir` /
`token_file` a tényleges `%ProgramData%`-ra mutasson.

Az `nssm.exe` **vendored** (`vendor/`): a nssm.cc gyakran 503-at ad, és egy
release build ne függjön tőle. A `vendor/README.md`-ben a verzió, a licenc
(public domain) és a sha256.

## Döntések, amiket érdemes tudni

- **Ikon és verzió-információ a binárisban.** A `crates/winres` build-script
  helper a `windres`-szel ágyazza be a `packaging/windows/ferrent.ico`-t
  (16–256 px, a GUI ikonjából) és egy VERSIONINFO blokkot mindhárom exébe,
  ugyanabból a forrásból, amit a telepítő használ. A `stage-artifacts.sh` a
  `.rsrc` szekció méretével ellenőrzi, hogy tényleg megtörtént: egy csendben
  kimaradt `windres` különben csak a Windows gépen tűnne fel.
- **Nincs cross OpenSSL.** A libtorrent a beépített RC4/SHA1-jét használja, ha
  nem talál OpenSSLt/GnuTLS-t/libgcryptot; a CMake-et ezért
  `CMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON`-nal hívjuk (különben a host Linux
  OpenSSL-jét próbálná belinkelni a Windows binárisba).
- **A Boost csak fejléc.** A `/usr/include`-ot nem szabad include-gyökérnek
  adni (a glibc `wchar.h`-ja elnyomja a mingw-ét), ezért a script a
  `boost/`-ot a **prefixbe** symlinkeli — így a `.pc` egy stabil, cache-elhető
  útra mutat (a `/tmp`-es út a következő konténerben nem létezne).
- **`libstdc++-6.dll` kell a daemon mellé.** A `cxx` bridge C++ kódja miatt a
  `ferrentd.exe` dinamikusan köti a C++ runtimet: a rustc a natív libeket
  `-Bdynamic` csoportba teszi, ezért a `-static` sem viszi be statikusan. A
  staging script a DLL-t is kimásolja (stripre 2,2 MB).
- **A binárisok strippelve** kerülnek a csomagba: 124 MB → 24 MB. A `.deb`
  ezzel szemben megtartja a szimbólumokat (a Bugsink a folyamatban oldja fel a
  kereteket); Windowson ez a trade-off a ZIP mérete miatt fordítva van, és az
  első VM-es futás dönti el, kell-e külön szimbólum-történet.
- **Import-kapu:** a `stage-artifacts.sh` elbukik, ha a `ferrentd.exe` a
  Windows API-n és a C++ runtime-on kívül bármit importál — egy dinamikus
  libtorrent így a buildben derül ki, nem a Windows gépen.
