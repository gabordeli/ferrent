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
| `crates/gui/src/main.rs:2287` | `Command::new("xdg-open")` | `ShellExecuteW` / `start` |
| `crates/gui/Cargo.toml:26` | `rfd` `default-features = false, features = ["gtk3"]` | Windows-on default features (IFileDialog) |
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

## Javaslat

**A v1.0.0-rc1 után az A (csak GUI, natív MSVC) az első lépés**, mert a munka
90%-a kész: a GUI futásidejű függőségei hordozhatók, a betűtípusok és a
nyelvi katalógus bundle-ölve vannak, a konfigmentésnek már van nem-Unix ága.

1. **`rfd` target-kapu** (`[target.'cfg(target_os = "linux")'.dependencies]` a
   gtk3-mal, egyébként default features) — enélkül Windows-on nincs fájlválasztó.
2. **`xdg-open` → `ShellExecuteW`** („Reveal in folder" / torrent megnyitása).
3. **Útvonalak**: `%ProgramData%\ferrent\{config.toml,token}`,
   `%APPDATA%\ferrent\gui.toml`; a token ACL-je a `0640 root:ferrent`
   megfelelője (`icacls`).
4. **Csomag**: ZIP + `ferrent.ico`; CI-ben `windows-latest` job (nincs
   libtorrent → nincs vcpkg, ~5 perc meleg cache-sel).
5. **A browseres PWA ma is működik Windows-ról** (0 munka) — ez a baseline,
   amit a natív GUI-nak felül kell múlnia.

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

## Döntési pontok (user)

- **Kell-e Windows-on daemon, vagy csak a GUI?** (az issue nyitott pontja) —
  ha csak a GUI, a munka ~2–3 nap; ha a daemon is, ~2 hét.
- **Van-e Windows gép/VM a teszteléshez?** Cross-build önmagában nem
  bizonyíték a futásra; a GUI megnyílása + csatlakozás egy Linux daemonhoz
  (TLS pinning, token) a valódi kapu.
- **ZIP elég, vagy MSI/winget?** Utóbbi csak akkor, ha a daemon is megy
  Windows-on.

## Becslés a v1.0.0-rc1 után

| Lépés | Munka |
|---|---|
| `rfd` kapu + `ShellExecuteW` + útvonalak | 1 nap |
| ZIP csomag + `ferrent.ico` + `windows-latest` CI job | 1 nap |
| Füstteszt Windows-on (indulás, csatlakozás, fájlválasztó) | 0,5 nap |
| **A: csak GUI összesen** | **2–3 nap** |
| libtorrent MSVC (vcpkg, statikus) zöldre hozása | 2–3 nap |
| 4 Unix-zárt forráshely + `rustix` kiváltása | 2 nap |
| Windows service (WinSW/NSSM) + token ACL + útvonalak | 1–2 nap |
| DiskGuard drive-betű + szabad hely | 1 nap |
| Windows tesztváltozatok a 11 Unix-kapus suite mellé | 2–3 nap |
| **C: daemon is összesen** | **~2 hét** |

## Nyitott pont

A **build út bizonyítása** (az acceptance 2. pontja) még hiányzik: ebben a
körben a `cargo check` a `ring`/`lib.exe` blokkolóig jutott, `.exe` nem
készült. A blokkoló nevesítve van (MSVC toolchain, illetve cross-compile-nál
`clang-cl` + `llvm-lib`); a következő lépés egy Windows hoston (vagy
`cargo-xwin`-nal) futó `cargo build -p ferrent-gui`, ami eldönti, hogy az A
tényleg 2–3 nap-e.
