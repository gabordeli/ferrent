# Windows 10/11 – kutatás (M13-T11)

Kérdés: készíthető-e Windows 10/11 bináris a Ferrentből, és milyen áron?
Nem implementáció: opció-mátrix, mért tények a repóból, becslés és javaslat.

Két **független** tengely van, és a legtöbb félreértés e kettő összemosásából
jön:

1. **Mit viszünk Windows-ra?** (a) csak a GUI-t (thin client, a daemon Linuxon
   marad), vagy (b) a daemont is.
2. **Hogyan épül?** natív MSVC Windows-on, cross-compile Linuxról, vagy WSL.

## Mért tények (2026-11-23)

- **A GUI-nak nincs C++ függősége.** A `crates/gui/Cargo.toml` futásidejű
  függőségei: `ferrent-protocol`, `ferrent-telemetry`, `ferrent-i18n`, iced
  0.13, `rfd`, `regex`, `serde`, `tokio`, `toml`, `toml_edit`. A
  `ferrent-daemon`/`ferrent-engine` **csak dev-dependency** → release buildhez
  nem kell libtorrent. A daemon viszont igen (`crates/engine/build.rs`:
  `pkg-config` `libtorrent-rasterbar >= 2.0`, majd `cxx_build` a
  `bridge.cpp`-re).
- **A GUI mégsem tiszta Rust.** `cargo check -p ferrent-gui --target
  x86_64-pc-windows-msvc` (linker nélkül) elhasal:
  `error: failed to run custom build command for ring v0.17.14` /
  `cc-rs: failed to find tool "lib.exe"`. A `ring` (a tonic `tls-ring`
  feature-én át) C-t és asm-t fordít, tehát **a típusellenőrzéshez is kell
  MSVC toolchain** (vagy `clang-cl` + `llvm-lib`). Kontroll: egy tiszta Rust
  crate ugyanerre a targetre átfordul (`cargo check -p ferrent-i18n
  --target x86_64-pc-windows-msvc` → 2,4 s, OK).
- **A kód Unix-zárt pontjai** (ezek Windows-on nem fordulnak le):

| Hol | Mi | Windows megfelelő |
|---|---|---|
| `crates/web/src/tls.rs:10` | `use std::os::unix::fs::PermissionsExt;` **modulszinten** | ACL (`icacls`) vagy `cfg`-kapu |
| `crates/daemon/src/diskguard/storage.rs:7,100` | `MetadataExt::dev()` → `st_dev` | volume serial (`GetVolumeInformationByHandleW`) vagy drive-betű |
| `storage.rs:121`, `rpc/mod.rs:244`, `autoadd.rs:653` | `rustix::fs::statvfs` | `GetDiskFreeSpaceExW` |
| `crates/daemon/src/hooks.rs:12,161` | `rustix::process::kill_process_group` + `process_group(0)` | Job object vagy `taskkill /T` |
| `crates/daemon/Cargo.toml:76` | `rustix` függőség (Unix-only crate) | `[target.'cfg(unix)'.dependencies]` |
| `crates/gui/src/main.rs` (`xdg-open`) | `Command::new("xdg-open")` | `explorer` (Windows) / `open` (macOS) — **kész** |
| `crates/gui/Cargo.toml:26` | `rfd` `gtk3` feature | **nem kell módosítani**: a feature Linux-only, a Win32 backend (IFileDialog) target-dependencyként mindig fordul |
| `main.rs:49`, `config/schema.rs:22` | `/etc/ferrent/config.toml`, `/etc/ferrent/token` | `%ProgramData%\ferrent\` |
| `crates/gui/src/config.rs:135` | `~/.config/ferrent/gui.toml` (`XDG_CONFIG_HOME`) | `%APPDATA%\ferrent\` |

- **Ami már most hordozható:** a leállításkezelés (`daemon/src/main.rs:253–263`
  `#[cfg(not(unix))]` ágon `tokio::signal::ctrl_c()`), a redb store, a
  rcgen/rustls TLS, a fastresume fájlok, a **TCP-only** RPC (nincs unix socket
  sehol), a GUI konfigmentése (`config.rs:116–127`, van `cfg(not(unix))` ág),
  a betűtípusok (`include_bytes!`: `theme.rs:178–180`, `icons.rs:12`) és a
  közös `ferrent-i18n` katalógus.
- **A tesztek nagyrészt Unix-ra vannak zárva:** 11 integrációs tesztfájl
  kezdődik `#![cfg(unix)]`-szel (`daemon/tests/*`, `gui/tests/*`,
  `cli/tests/cli.rs`). Windows-on tehát a suite szinte üresen fut — a
  „zöld CI" ott keveset bizonyít.
- **Minden CI job `ubuntu-24.04`** (`.github/workflows/ci.yml`); Windows
  runner ma nincs a láncban.
- **Méretek referenciaként:** 46 526 sor Rust / 99 fájl (spike-ok nélkül);
  `.deb` 3,79 MB, telepítve 14,9 MB; AppImage 43 MB; teljes release build
  konténerben 2 m 12 s (daemon-only). libtorrent 2.0.11, statikusan
  (`packaging/debian/build.sh:18,111`).

## Opciók

| | **A. Csak GUI, natív MSVC** | **B. Csak GUI, cross-compile** | **C. Daemon is Windows-on** | **D. WSL-only** |
|---|---|---|---|---|
| Mit ad | `ferrent-gui.exe`, Linux daemonhoz csatlakozik | ugyanaz, Linuxról buildelve | `ferrentd.exe` + GUI egy gépen | a Linux GUI WSLg-vel |
| Build | Windows gép + MSVC (a `ring` miatt kötelező) | `cargo-xwin` + `clang-cl`/`llvm-lib` (a `ring` miatt) | MSVC + libtorrent vcpkg-ból (boost, OpenSSL, zlib) | nincs |
| libtorrent | nem kell | nem kell | kell: MSVC build, cold ~25–40 perc (cache-elhető) | nem kell |
| Kódmódosítás | `rfd` target-kapu, `xdg-open`, config/token útvonalak | ugyanaz | + a 4 Unix-zárt forráshely, `rustix` kiváltása, Windows service | nincs |
| Tesztelés | valódi Windows gép/VM kell | **a futáshoz is** Windows kell | Windows + drive-betűs zónák | WSL |
| Csomagolás | ZIP (`.ico` kell; ma csak PNG van) | ZIP | ZIP/MSI (WiX) vagy winget + WinSW/NSSM service | — |
| Munka (első verzió) | **2–3 nap** | 1–2 nap + Windows a teszthez | **~2 hét** | 0 nap |
| Kockázat | minimális; a felhasználónak Linux daemon kell | a „cross-build, nem futtattam" csapda | libtorrent MSVC + DiskGuard drive-szemantika | nem valódi Windows app |

## A forrás-port (2026-11-23)

A két tengely közül az **all-in-one** mellett döntött a user: Windows-on a
daemon, a CLI és a GUI is kell, egy csomagban. A forrás szintű port elkészült;
a mérés alapja egy helyi cross-check hurok, ami Windows **nélkül** is fog:

```bash
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64     # egyszer
rustup target add x86_64-pc-windows-gnu
cargo check --workspace --exclude ferrent-daemon --target x86_64-pc-windows-gnu
cargo check -p ferrent-daemon --no-default-features --target x86_64-pc-windows-gnu
```

A `--no-default-features` kell, mert a daemon default feature-je a
`libtorrent` (annak a Windows buildje külön munka); a mock engine viszont a
daemon **saját** kódját teljesen típusellenőrzi.

Ezen a hurkon a kiindulás **9 hiba** volt, a végállapot **0**:

| Crate | Kiinduló hiba | Javítás |
|---|---|---|
| `ferrent-cli` | 0 | már hordozható volt |
| `ferrent-web` | 2 (`PermissionsExt`, `from_mode`) | `cfg(unix)` kapu a kulcs `0600`-jára |
| `ferrent-daemon` | 6 (`st_dev`, 3× `statvfs`, `kill_process_group`, `rustix`) | `display` alatti platform-réteg |
| `ferrent-gui` | 1 (`PlatformSpecific::application_id`) | `window_platform_specific()` per-platform |

A daemon platform-rétege (`crates/daemon/src/diskguard/storage.rs`):

- `disk_stat` / `disk_stat_nearest`: Unix `statvfs`, Windows
  `GetDiskFreeSpaceExW` (`free` = a hívó által írható, `used` = `total − total_free`);
- `device_id`: Unix `st_dev`, Windows a drive **volume serial**ja
  (`GetVolumeInformationW`), a nem létező útvonalon `None` — ugyanaz a
  szemantika, mint Unixon, így a `device_id_nearest` ott is felfelé lép;
- `mount_table`: Windows-on `GetLogicalDrives` + volumeinfo → egy bejegyzés
  drive-onként (`C:\`, fstype a valódi fájlrendszer-név), így a DiskGuard
  zónái drive-okra is működnek, nem üres táblára;
- `hooks::kill_tree`: Unix process-group `SIGKILL`, Windows `taskkill /T /F`.

A `rustix` innentől `[target.'cfg(unix)'.dependencies]`, a Windows-oldal
`windows-sys` 0.59 (már a lockfile-ban volt, nincs új letöltés).

**Ami még hátra van:** `ferrent.ico` (a ZIP-ben még nincs), a GUI
`%APPDATA%\ferrent\gui.toml` útvonala, a daemon `%ProgramData%` alapértékei
(a service-generált config már ott él), és a 11 `#![cfg(unix)]` teszt Windows
változatai. A libtorrent cross-build, a ZIP és a service wrapper **kész**
(`packaging/windows/`); a service a daemont NSSM-mel futtatja, és
`AppStopMethodConsole`-lal adja át a Ctrl-C-t, hogy a fastresume mentés
lefusson.

### CI: `windows-cross` a saját runneren

A runner az egyetlen CI-host, a Windows VM kézi teszt-gép lesz, ezért a
Windows binárisok **cross-buildel** készülnek (`stage: quality`, a
`.gitlab-ci.yml`-ben):

- a job a `rust:1.100-slim` konténerben telepíti a mingw-t, a CMake-et, a
  Boost fejléceket és a `x86_64-pc-windows-gnu` targetet (a hoston is ott van
  a kézi buildhez: `infra` → `gitlab_runner_toolchain` role);
- `packaging/windows/build-libtorrent.sh` cross-buildeli a libtorrentet egy
  **cache-elt prefixbe** (`$HOME/.cache/ferrent-lt-win`), így a CMake build
  nem fut minden pipeline-ban; utána egy `cargo build` adja a három
  binárist (daemon a valódi engine-nel, nem mock);
- `packaging/windows/stage-artifacts.sh` strippel és kimásolja a
  `libstdc++-6.dll`-t (a `cxx` bridge miatt a daemon dinamikusan köti a C++
  runtimet), majd a `make-zip.sh` összeállítja a
  `Ferrent-<ver>-windows-x86_64.zip`-et (binárisok + DLL + `nssm.exe` +
  `service.ps1` + minta config); mindkettő artifact;
- a `file` kimenete az első futásnál „PE32+ executable (console)" volt: a
  GUI release buildje `windows_subsystem = "windows"`-t kapott, hogy ne
  nyíljon konzolablak (debugban marad, a pániküzenet miatt).

## Javaslat

**All-in-one (a user döntése): daemon + CLI + GUI, egy Windows csomag.**

1. ~~`rfd` target-kapu~~ — **nem kell**, a `gtk3` feature Linux-only.
2. ~~`xdg-open`~~ — `explorer`/`open` ágra váltva.
3. **Útvonalak**: `%ProgramData%\ferrent\{config.toml,token}`,
   `%APPDATA%\ferrent\gui.toml`; a token ACL-je a `0640 root:ferrent`
   megfelelője (`icacls`).
4. **Csomag**: ZIP (majd MSI/winget) + `ferrent.ico`; CI-ben `windows-latest`
   job, cache-elt vcpkg-val a libtorrenthez.
5. **A browseres PWA ma is működik Windows-ról** (0 munka) — ez a baseline,
   amit a natív csomagnak felül kell múlnia.

**A C (daemon is) külön döntés**, és csak akkor éri meg, ha valaki Windows-ról
akar seedelni: a DiskGuardnak drive-betűkkel kell zónáznia, a szabad helyet
`GetDiskFreeSpaceExW`-vel mérni, a hookokat Job objectben ölni, a leállást
pedig a service wrappernek `Ctrl-C`-ként kell átadnia (NSSM
`AppStopMethodConsole` / WinSW), hogy a fastresume mentés lefusson. A `rustix`
helyére `windows-sys` kell, és a 11 `#![cfg(unix)]` teszt mellé Windows
változatok.

**A B (cross-compile) csak kényelmi út**: a `ring` miatt `clang-cl` +
`llvm-lib` kell, és a build sikerétől még senki nem látta a Direct3D 12
renderelést — a `windows-latest` CI job (A) őszintébb.

**A D (WSL) nem app**, csak fallback: a Linux GUI WSLg-vel elindul, de a
felhasználónak WSL kell.

## Döntési pontok

- ~~Kell-e Windows-on daemon, vagy csak a GUI?~~ **Döntés (2026-11-23):
  all-in-one** — daemon + CLI + GUI egy csomagban, ~2 hét.
- **Van-e Windows gép/VM a teszteléshez?** Cross-check (mingw) a forrásra
  elég, de a futásra nem: a GUI megnyílása + csatlakozás egy daemonhoz
  (TLS pinning, token) a valódi kapu.
- **ZIP elég, vagy MSI/winget?** All-in-one-nál a ZIP csak az első lépés; a
  service telepítés miatt MSI (WiX) vagy winget a cél.

## Becslés a v1.0.0-rc1 után

| Lépés | Munka | Állapot |
|---|---|---|
| Forrás-port (9 hiba → 0) + cross-check hurok | 1 nap | **kész** (2026-11-23) |
| CI cross-build (CLI + GUI + daemon) a runneren | 1 nap | **kész** (2026-11-23) |
| libtorrent cross-build mingw-vel (beépített crypto) | 2–3 nap | **kész** (2026-11-23, ~4 perc) |
| ZIP csomag (binárisok + DLL + NSSM + service.ps1) | 1–2 nap | **kész** (2026-11-23) |
| Windows service (NSSM) + `Ctrl-C` átadás | 1–2 nap | **kész** (2026-11-23) |
| `ferrent.ico` + `%APPDATA%`/`%ProgramData%` alapértékek + token ACL | 1 nap | hátra |
| Windows tesztváltozatok a 11 Unix-kapus suite mellé | 2–3 nap | hátra |
| **Összesen (all-in-one)** | **~2 hét** | |

## Nyitott pont

A **build út bizonyítása** (az acceptance 2. pontja) még hiányzik: ebben a
körben a forrás típusellenőrzése zöld (`x86_64-pc-windows-gnu`), de `.exe`
nem készült. Két ismert blokkoló van: a `ring` MSVC-t kér a C-jéhez, a
libtorrentnek pedig nincs Windows-on kész `.lib`-je (vcpkg build kell).
A következő lépés egy Windows hoston (vagy `cargo-xwin`-nal) futó
`cargo build`, ami eldönti, hogy a becslés tartható-e.
