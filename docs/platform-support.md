# Platform-támogatás

Ez a mátrix mondja meg, melyik komponens melyik platformon mit tud, és mi a
bizonyíték. A "támogatott" szó itt **futásban igazolt** állapotot jelent, nem
buildelhetőséget.

## Státuszok

| Státusz | Jelentés |
|---|---|
| **támogatott** | csomagolt (`.deb`/registry), CI-tesztelt, és élesben vagy ellenőrzött környezetben fut |
| **előnézet** | buildel és csomagol (ZIP/NSIS), de nincs futásidejű igazolás; hibajegyet ér, de nem release-blokkoló |
| **nem** | nincs build; legfeljebb kódbeli ág |

## Mátrix (2026-11-24)

| Komponens | Linux (Debian/Ubuntu, x86_64/arm64) | Windows 10/11 | macOS |
|---|---|---|---|
| `ferrentd` daemon | **támogatott** — `.deb` + systemd, éles a .27-en (`1.0.0~rc1-1`), a CI teszteket futtat | **előnézet** — ZIP + NSIS, `x86_64-pc-windows-gnu` cross-build a CI-ban; service/config-út/token-ACL futásban **méretlen** | **nem** — nincs build |
| `ferrent-cli` | **támogatott** — `.deb`, a CI CLI-integrációs tesztjei zöldek | **előnézet** — a ZIP része; a CLI-tesztek `#![cfg(unix)]` miatt Windows-on nem futnak | **nem** |
| `ferrent-gui` | **támogatott** — `.deb` + AppImage | **előnézet** — buildel és a ZIP része; a GUI futás Windows-on méretlen | **nem** — csak `open_in_file_manager` ág |
| `ferrent-web` gateway | **támogatott** — külön bináris, systemd alatt | **nem** — nincs a Windows ZIP-ben | **nem** |
| Deluge import | **támogatott** — `import-deluge` + GUI | **kikapcsolva** — a daemon `import_enabled=false`-t hirdet a VM-tesztig | **nem** |

**iOS kliens:** külön kutatás (`docs/research/ios.md`); 2026-11-24-én
**parkol**, nincs build. A cél App Store-feltölthető app, közvetlen `ferrentd`
RPC-kapcsolattal és APNs push-sal (a Web Push iOS 16.4+ / home-screen PWA,
ezért az iPhone 7 nem tudja).

## Szabályok

1. **A platform-képességet a daemon hirdeti.** Az `import_enabled` a
   `GetStatusResponse`-ban van; a GUI a daemon válaszára épít, a saját
   build-célja nem dönt (`1c88460`).
2. **"Támogatott" csak futásidejű smoke után.** A Windows előnézetből akkor
   lesz támogatott, ha a service telepítés/indítás/leállítás, a config-út, a
   token-ACL és a Linux daemonhoz kapcsolódó GUI egy VM-en (vagy
   `windows-latest` runneren) igazolva van.
3. **A macOS nem cél v1.0-ig.** A kódbeli ágak megmaradnak, de nincs build és
   nincs CI; ne hivatkozzunk rá támogatottként.
4. **A platform-specifikus kód helye:**
   - `crates/platform` — közös: config könyvtár, `~/` feloldás, titok-írás;
   - `crates/daemon/src/platform.rs` — a daemon telepítési layoutja
     (`/etc/ferrent` vs `%ProgramData%\ferrent`), token-ACL, import-kapu;
   - `crates/daemon/src/diskguard/storage.rs` — lemez-API (`statvfs` /
     `GetDiskFreeSpaceExW`, volume serial, mount table).

## Miért nem elég a cross-build

A `windows-cross` CI job `cargo build --release` + ZIP/NSIS, **teszt és
futtatás nélkül**; a 13 integrációs tesztfájl `#![cfg(unix)]`, a GitHub
workflow-kon nincs Windows/macOS job. A platform-kód tehát fordul, de a
viselkedése (ACL, service-stop, meghajtóbetűk, szabad hely) nincs igazolva —
ezért "előnézet", nem "támogatott".

## Kapcsolódó mérések

- `docs/research/windows.md` — az opció-mátrix, a cross-build és a hátralévő
  lépések.
