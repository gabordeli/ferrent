# Ferrent – roadmap

> Állapot: 2026-11-19, **terv** (kód még nincs a repóban).
> Architektúra: [ARCHITECTURE.md](ARCHITECTURE.md).
> Sorrend: **MVP (v0.1)** → **v1.0**. A taskok nem időbecslések, hanem lezárt, ellenőrizhető egységek.

**Jelölések:** 🤖 agent-ready (önállóan kipipelhető, nincs külső döntés) · ⚠️ döntést igényel · 🔬 mérés/benchmark

---

# MVP (v0.1) – „nem döglik meg 4 000 torrentnél”

**Definíció:** Egy Debian szerveren futó `ferrentd`, amely 4 000 torrentet visz, CLI-ról és natív GUI-ból vezérelhető, és újraindítás után nem esik recheck-be.

**Done when**

- [ ] `ferrentd` betölt 4 000 torrentet recheck nélkül; RSS ≤ 1 GB; indulás ≤ 30 s (R1, R3).
- [ ] A `libtorrent` dedikált szálon fut; a tokio száron nincs blokkoló hívás (R2, mérve).
- [ ] `ferrent-cli`: add/list/show/pause/resume/remove + delta `watch` (JSON kimenettel).
- [ ] `ferrent-gui`: 4 000 soros virtualizált tábla ≥ 60 FPS, TLS-sel csatlakozik, a műveletek működnek.
- [ ] `.torrent` hozzáadás nyers bájton át megy (távoli daemon is).
- [ ] Legalább egy valós torrent végigfut a daemonon, és a restart nem okoz rechecket.
- [ ] Az eredmények `docs/measurements/mvp.md`-ben rögzítve; `v0.1.0` tag.

**Non-goals (MVP)**

- Csomagolás (`.deb`, APT, Docker), `import-deluge`, `*arr` kompatibilitás.
- Webhookok, auto-remove, VPN-kötés, Prometheus.
- Windows/macOS build, mobil, tracker whitelist.
- Több motor implementáció (csak a trait + `libtorrent`).

**Agent-ready MVP taskok (ebben a sorrendben pipeline-olhatók):**
`M0-T1` → `M0-T2` → `M0-T3` → `M1-T1` → `M1-T4`

---

## M0 – Workspace és engine alapok

**Done when:** a workspace buildel; a `TorrentEngine` trait + mock tesztelt; a `libtorrent` bridge egy torrentet végigvisz; a 4 000 torrentes harness mér és reportol.

**Non-goals:** daemon, RPC, GUI, csomagolás.

**Függ:** –

### M0-T1 – Cargo workspace, csomagnevek, CI váz 🤖

**Scope:** workspace gyökér, öt crate üres vázza, közös lint/format, crate-enkénti CI jobok, licenc-döntés helye.

**Acceptance**

- [ ] `cargo build --workspace` és `cargo test --workspace` tiszta checkouton zöld.
- [ ] Csomagnevek: `ferrent-protocol`, `ferrent-engine`, `ferrent-daemon`, `ferrent-cli`, `ferrent-gui`; binárisok: `ferrentd`, `ferrent-cli`, `ferrent-gui`.
- [ ] `cargo build -p ferrent-gui -p ferrent-cli` C++/libtorrent eszközlánc nélkül is lefut.
- [ ] CI: `fmt --check`, `clippy -D warnings`, `test`, crate-enkénti path-szűrés.
- [ ] Egységes `edition` és `rust-version` a workspace `Cargo.toml`-ban; licenc placeholder rögzítve.

**Files:** `Cargo.toml`, `crates/*/Cargo.toml`, `rustfmt.toml`, `.github/workflows/ci.yml`

### M0-T2 – `TorrentEngine` trait + in-memory mock 🤖

**Scope:** motor-agnosztikus trait, determinisztikus mock implementáció, esemény-delta szerződés.

**Acceptance**

- [ ] A trait lefedi: add/pause/resume/remove/list/status/settings/event-stream.
- [ ] A publikus API-ban nincs `libtorrent`-típus (csak saját típusok).
- [ ] A mock minden metódust implementál; teszteli az állapotátmeneteket.
- [ ] Az esemény-stream deltákat ad (added, state-changed, progress, removed), stabil sorrendben.
- [ ] `cargo test -p ferrent-engine` zöld, C++ függőség nélkül.

**Files:** `crates/engine/src/lib.rs`, `crates/engine/src/trait.rs`, `crates/engine/src/mock.rs`, `crates/engine/tests/`

### M0-T3 – `libtorrent` + `cxx` bridge 🤖

**Scope:** session wrapper, add/pause/resume/remove, státusz-snapshot, alert-polling, Debian build-deps dokumentálása.

**Acceptance**

- [ ] `cargo build -p ferrent-engine --features libtorrent` Debian 12-n a dokumentált apt-csomagokkal lefut.
- [ ] Egy valós `.torrent` hozzáadható, letölt/metadata érkezik, pause/resume/remove működik.
- [ ] Az alert-loop `TorrentEngine` eseményekké alakít; a csatorna nem nő korlátlanul.
- [ ] A session dedikált szálon fut; tokio kontextusból nincs blokkoló hívás (teszt).
- [ ] `docs/adr/0001-libtorrent-version.md`: 1.2 vs 2.0 döntés méréssel (RSS, CPU, indulás).

**Files:** `crates/engine/build.rs`, `crates/engine/src/libtorrent/`, `docs/adr/0001-libtorrent-version.md`

### M0-T4 – 4 000 torrentes load harness

**Scope:** szimulált/valós torrent-generátor, mérés (RSS, CPU, indulás, leállás, fd, alert/sec), riport.

**Acceptance**

- [ ] `cargo run -p ferrent-engine --example loadtest --features libtorrent -- --torrents 4000 --mode seed`.
- [ ] Méri és JSON-ban írja: RSS, idle/aktív CPU, indulási és leállási idő, nyitott fd, alert/sec.
- [ ] Markdown összefoglaló `docs/measurements/m0-loadtest.md`-be kerül.
- [ ] Az eredmény összevetve az ARCHITECTURE §6 büdzsével; eltérés indoklással.
- [ ] A harness újrafuttatható (seed, fix paraméterek) és a baseline rögzítve.

**Files:** `crates/engine/examples/loadtest.rs`, `docs/measurements/m0-loadtest.md`, `docs/measurements/m0-loadtest.json`

---

## M1 – Daemon, perzisztencia, protokoll, CLI

**Done when:** a `ferrentd` 24/7 fut; a state mirror kiszolgálja a klienseket; a restart nem recheckel; a gRPC delta-stream és a CLI működik.

**Non-goals:** GUI, csomagolás, `*arr`, import.

**Függ:** M0

### M1-T1 – Perzisztencia réteg (redb/SQLite) 🤖 ⚠️

**Scope:** `StateStore` trait + implementáció; fastresume, címkék, beállítások, statisztikák; dirty tracking; crash-safe írás.

**Acceptance**

- [ ] `StateStore` API: get/put/delete/list/batch, verziózott séma.
- [ ] Csak a módosult rekordok íródnak; teszt bizonyítja, hogy nincs teljes újraírás.
- [ ] `kill -9` írás közben → az DB újraindítás után olvasható (teszt).
- [ ] 4 000 rekord betöltése ≤ 2 s; batch write ≤ 100 ms (célérték).
- [ ] `docs/adr/0002-persistence.md`: redb vs SQLite döntés méréssel.

**Files:** `crates/daemon/src/store/`, `crates/daemon/tests/crash_recovery.rs`, `crates/daemon/src/bin/crash_writer.rs`, `docs/adr/0002-persistence.md`

### M1-T2 – State mirror + alert-loop

**Scope:** `libtorrent` szál → `mpsc` → memóriabeli állapottábla; szűrés/rendezés/lapozás; lock-mentes olvasási út.

**Acceptance**

- [ ] A lista-kéréseket a mirror szolgálja ki, `engine` hívás nélkül.
- [ ] p95 lekérdezés ≤ 50 ms 4 000 torrentnél (benchmark).
- [ ] `std::sync::Mutex` nincs `.await` alatt tartva (clippy + review).
- [ ] A delta-feldolgozás memóriája korlátos; nincs eseményenkénti allokációs szivárgás.
- [ ] Leállításkor a mirror draineli az alert-eket és kiírja a dirty állapotot.

**Files:** `crates/daemon/src/state.rs`, `docs/measurements/m1-mirror.md`

### M1-T3 – gRPC protokoll, TLS, auth ⚠️

**Scope:** `.proto`, szerver + kliens stub, lapozott/szűrt/rendezett lekérdezés, delta-stream, önaláírt TLS + TOFU, token auth.

**Acceptance**

- [ ] `proto/ferrent.proto` verziózott; mezőszám nem újrahasznált; breaking-change ellenőrzés CI-ban.
- [ ] Kliens TLS-en át tud add/list/pause/resume/remove műveletet.
- [ ] Delta-stream: inaktív torrent 60 s alatt 0 üzenet; aktív ~1 Hz.
- [ ] Rossz token → `UNAUTHENTICATED`; pin-elt ujjlenyomat eltérés → egyértelmű hiba.
- [ ] `docs/adr/0004-transport.md`: gRPC vs WebSocket döntés indoklással.

**Files:** `crates/protocol/proto/ferrent.proto`, `crates/daemon/src/rpc/`, `crates/protocol/src/client.rs`

### M1-T4 – `ferrent-cli` 🤖

**Scope:** config profilok, add/list/show/pause/resume/remove, `watch` delta mód, JSON kimenet.

**Acceptance**

- [ ] Parancsok TLS-en, tokennel (config/env) működnek.
- [ ] `--json` kimenet dokumentált és stabil (mezőnevek verziózva).
- [ ] `ferrent-cli watch` streamel; Ctrl-C tisztán leáll.
- [ ] Integrációs teszt in-process daemon ellen.
- [ ] Súgó és exit code-ok dokumentálva.

**Files:** `crates/cli/src/main.rs`, `crates/cli/src/commands/`, `crates/cli/tests/`

---

## M2 – Thin client MVP

**Done when:** a GUI csatlakozik, 4 000 soros virtualizált táblát mutat, a műveletek működnek, és a keretrendszer-döntés mérve van.

**Non-goals:** WebUI, témák, grafikus beállítás-szerkesztő, mobil.

**Függ:** M1

### M2-T1 – GUI keretrendszer spike: `iced` vs `Tauri` ⚠️ 🔬

**Scope:** 4 000 soros virtualizált tábla prototípus mindkettőben; RAM, FPS, hidegindulás, bináris méret, fejlesztési súrlódás mérése.

**Acceptance**

- [ ] Mindkét prototípus virtualizáltan renderel 4 000 sort (csak a látható sorok).
- [ ] Mérve: RSS, görgetési FPS, hidegindulás, bináris méret.
- [ ] `docs/adr/0003-gui-framework.md` rögzíti a döntést számokkal.
- [ ] A vesztes prototípus nem kerül a release binárisba.

**Files:** `crates/gui/spikes/`, `docs/adr/0003-gui-framework.md`

### M2-T2 – GUI shell, kapcsolat, TLS pinning

**Scope:** app shell, szerverprofilok, TOFU fingerprint dialógus, újracsatlakozás, státuszsáv.

**Acceptance**

- [ ] Első csatlakozáskor mutatja az ujjlenyomatot; elfogadás után tárolja; eltéréskor figyelmeztet.
- [ ] Daemon-restart után az app újraindítás nélkül újracsatlakozik.
- [ ] A token biztonságosan tárolva (keyring vagy 0600 config), nem kerül logba.
- [ ] Hibák a UI-ban jelennek meg, nem panic.

**Files:** `crates/gui/src/app.rs`, `crates/gui/src/connection.rs`

### M2-T3 – Virtualizált torrent tábla + műveletek

**Scope:** oszlopok, server-side szűrés/rendezés, többszörös kijelölés, kontextusmenü, részletek panel.

**Acceptance**

- [ ] 4 000 sor görgetése ≥ 60 FPS; csak a viewport sorai kérdezhetők le.
- [ ] Rendezés/szűrés szerveroldali kérést indít; nincs teljes lista letöltés.
- [ ] `.torrent` hozzáadás nyers bájtokkal megy (távoli daemonon is működik).
- [ ] Pause/resume/remove optimista UI + hiba esetén visszaállítás.
- [ ] Gyakori műveletek billentyűparancsról; dokumentálva.

**Files:** `crates/gui/src/table.rs`, `crates/gui/src/actions.rs`

### M2-T4 – MVP end-to-end acceptance

**Scope:** Debian + 4 000 torrent + GUI + CLI; mérés, soak, dokumentálás, `v0.1.0` tag.

**Acceptance**

- [ ] R1/R2/R3 mérve és rögzítve (`docs/measurements/mvp.md`).
- [ ] 24 órás soak: nincs crash; RSS-növekedés < 10%.
- [ ] Restart recheck nélkül; indulás ≤ 30 s.
- [ ] GUI másik gépről TLS-en csatlakozik.
- [ ] Ismert korlátok dokumentálva; `v0.1.0` tag.

**Files:** `docs/measurements/mvp.md`, `README.md`

---

# v1.0 – telepíthető, migrálható, üzemeltethető

**Definíció:** `apt install ferrentd` egy tiszta Debianon; a meglévő Deluge-állomány átemelhető; a *arr stack és a VPN-es homelab használat kényelmes; a rendszer soak-tesztelt.

**Done when**

- [ ] Aláírt APT repo + `systemctl` + Docker image; friss telepítés dokumentált.
- [ ] `import-deluge` 4 000 torrentet visz át, újraellenőrzés nélkül (vagy dokumentált fallbackkel).
- [ ] Sonarr/Radarr add + list működik az emulált qBittorrent API-n.
- [ ] VPN-interfész kötés leak-teszttel bizonyítva; `/metrics` Grafana-kompatibilis.
- [ ] 72 órás soak + `kill -9` recovery zöld; nincs fd/RSS szivárgás.
- [ ] Dokumentáció (telepítés, CLI, API, migráció) és `v1.0.0` kiadás.

**Non-goals (v1.0)**

- Mobil kliens, média-streaming, több daemon klaszter.
- Saját torrent motor, Deluge plugin ABI.
- Windows/macOS daemon.

---

## M3 – Csomagolás és terjesztés

**Done when:** a daemon egy `.deb`-ből telepíthető Debian 12/13-on és a támogatott Ubuntukon; az APT repo és a Docker image automatikusan készül tagre.

**Non-goals:** kliens store-terjesztés (Snap/Flathub jóváhagyás), Windows installer.

**Függ:** M2

### M3-T1 – `cargo-deb` + systemd unit

**Scope:** `packaging/debian`: unit, `/etc/ferrent/config.toml`, `ferrent` rendszerfelhasználó, postinst/prerm, man page.

**Acceptance**

- [ ] `cargo deb -p ferrent-daemon` előállítja a binárist, unitot, configot, man page-et.
- [ ] Friss Debian 12-n telepítés után `systemctl start ferrentd` működik; a `ferrent` user létrejön.
- [ ] `apt remove` megtartja a `/var/lib/ferrent`-et; `purge` törli (dokumentálva).
- [ ] A csomagban nincs secret; a config jogosultsága 0640, tulajdonos `root:ferrent`.

**Files:** `packaging/debian/`, `crates/daemon/Cargo.toml` (`[package.metadata.deb]`)

### M3-T2 – Statikus `libtorrent` build + több disztrós `.deb` ⚠️

**Scope:** statikus link, build konténer, CI mátrix (Debian 12/13, Ubuntu 24.04), arm64.

**Acceptance**

- [ ] `ldd ferrentd` nem mutat `libtorrent` függőséget; csak a vállalt glibc/libstdc++ baseline.
- [ ] Ugyanaz a `.deb` települ és fut Debian 12, 13 és Ubuntu 24.04 konténerben.
- [ ] A bináris méret dokumentálva; a build konténerben reprodukálható.
- [ ] arm64 build (cross vagy natív) lefut és smoke-tesztet kap.
- [ ] `docs/adr/0005-min-debian.md`: minimum Debian verzió döntés.

**Files:** `packaging/debian/build.sh`, `.github/workflows/release-deb.yml`, `docs/adr/0005-min-debian.md`

### M3-T3 – APT repo pipeline

**Scope:** tag → build → GPG-sign → index → GitHub Pages; DEB822 `Signed-By`; régi verziók megtartása.

**Acceptance**

- [ ] Tag push után `apt update && apt install ferrentd` működik tiszta Debianon.
- [ ] A kulcs dokumentált; `Signed-By=/etc/apt/keyrings/ferrent.gpg`; nincs `apt-key`.
- [ ] Az index checksumokat tartalmaz; legalább az előző verzió elérhető marad.
- [ ] Hiányzó secret esetén a pipeline egyértelműen elbukik.

**Files:** `.github/workflows/release-apt.yml`, `packaging/apt/`

### M3-T4 – Docker image + AppImage/Flatpak

**Scope:** headless daemon image a GHCR-re; kliens AppImage vagy Flatpak.

**Acceptance**

- [ ] `docker run` volume-vel (`/var/lib/ferrent`) és publikált RPC porttal működik.
- [ ] Az image non-rootként fut; healthcheck definiált.
- [ ] Az AppImage egy nem Debian-alapú disztrón (Fedora konténer) elindul és csatlakozik.
- [ ] Mindkettőhöz telepítési doksi.

**Files:** `packaging/docker/Dockerfile`, `packaging/appimage/`

---

## M4 – Ökoszisztéma és automatizáció

**Done when:** a Deluge-ból át lehet költözni; a *arr stack és a VPN-es homelab kiszolgálható; a pluginok helyét beépített szabályok és webhookok veszik át.

**Non-goals:** teljes Deluge RPC emuláció, plugin ABI, torrent kereső/indexer.

**Függ:** M3

### M4-T1 – `import-deluge`

**Scope:** Deluge `state` könyvtár beolvasása (`.torrent` + `.fastresume` + címkék), dry-run, idempotens import.

**Acceptance**

- [ ] `ferrent-cli import-deluge --state-dir ... --dry-run` listázza a talált torrenteket.
- [ ] 4 000 torrent importja lefut; a címkék megőrződnek; a duplikátumok kimaradnak.
- [ ] Kompatibilis fastresume esetén nincs recheck; ellenkező esetben dokumentált fallback.
- [ ] Újrafuttatás nem duplikál (idempotens).
- [ ] Kis valós Deluge-fixture-rel tesztelve.

**Files:** `crates/cli/src/import_deluge.rs`, `tests/fixtures/deluge-state/`

### M4-T2 – qBittorrent Web API emuláció (*arr)

**Scope:** auth/login, `torrents/add`, `torrents/info`, kategóriák, pause/resume/delete; opt-in végpont.

**Acceptance**

- [ ] Sonarr/Radarr hozzáad egy torrentet és látja a queue-ban az emulált endpointon.
- [ ] Az API-verzió endpoint válaszol; cookie/header auth működik.
- [ ] Kategória → mentési útvonal mapping.
- [ ] Szerződés-tesztek rögzített kérésekkel.
- [ ] Feature flag mögött; dokumentálva.

**Files:** `crates/daemon/src/compat/qbittorrent.rs`, `crates/daemon/tests/compat/`

### M4-T3 – Automatizáció: címkék, auto-remove, hookok/webhookok

**Scope:** címke/kategória szabályok mentési útvonallal; arány/idő/seeding limit; shell hook + JSON webhook retry-jal.

**Acceptance**

- [ ] A szabályok TOML-ban konfigurálhatók; reload viselkedése dokumentált.
- [ ] Az auto-remove betartja a limiteket; dry-run mód elérhető.
- [ ] Webhook: timeout, retry, log; nem blokkolja a torrent szálat.
- [ ] A hook környezeti változók dokumentálva; gyakori példák.
- [ ] Szabály-kiértékelés unit tesztelve.

**Files:** `crates/daemon/src/automation/`

### M4-T4 – VPN interfész-kötés + Prometheus `/metrics`

**Scope:** `outgoing_interfaces`/`listen_interfaces` kötés, link-figyelés, szüneteltetés interfész-vesztéskor; opcionális metrics endpoint.

**Acceptance**

- [ ] `wg0`-hoz kötve, az interfész leállításakor nincs kimenő forgalom (`ss`/`tcpdump` bizonyíték).
- [ ] Visszatéréskor automatikus resume, konfigurálható grace period.
- [ ] `/metrics`: torrentszám, le/fel sebesség, peer-ek, fd, RSS; Grafana scrape működik.
- [ ] A metrics endpoint opcionális és auth-konfigurálható.

**Files:** `crates/daemon/src/netbind.rs`, `crates/daemon/src/metrics.rs`

---

## M5 – Hardening és 1.0 kiadás

**Done when:** a rendszer soak- és crash-tesztelt, a biztonsági és teljesítmény-regresszió kapuzott, a dokumentáció teljes, a `v1.0.0` kiadva.

**Non-goals:** új nagy funkció; csak hardening, mérés, dokumentáció, release.

**Függ:** M4

### M5-T1 – Soak, crash recovery, robustness

**Scope:** 72 órás soak 4 000 torrenttel; `kill -9` recovery; sérült DB rekord; malformed protokoll üzenet.

**Acceptance**

- [ ] 72 h soak: nincs crash; RSS-növekedés < 10%; nincs fd-szivárgás.
- [ ] `kill -9` után restart recheck nélkül; az állapot konzisztens.
- [ ] Sérült DB rekord → a daemon indul, a rossz rekordot karanténba teszi.
- [ ] Malformed RPC üzenet → hiba, nem panic.
- [ ] Eredmények `docs/measurements/v1-soak.md`-ben.

**Files:** `tests/soak/`, `docs/measurements/v1-soak.md`

### M5-T2 – Biztonság + teljesítmény-regresszió

**Scope:** auth/TLS review, függőség-audit, nightly perf harness a büdzséhez képest; threat model.

**Acceptance**

- [ ] Nincs secret a logokban; a token rotáció működik.
- [ ] `cargo audit` tiszta vagy dokumentált kivételekkel.
- [ ] A perf harness baseline-hoz hasonlít; >10% regresszió esetén a nightly fail.
- [ ] TLS konfig és pinning tesztelve; rövid threat model doksi.
- [ ] `docs/security.md` kitöltve.

**Files:** `.github/workflows/perf.yml`, `docs/security.md`

### M5-T3 – Dokumentáció + `v1.0.0` release

**Scope:** README quickstart, telepítés, CLI/API referencia, migrációs útmutató, release notes, tag.

**Acceptance**

- [ ] README: `apt install` → systemd → GUI csatlakozás 10 percen belül.
- [ ] API referencia a proto-ból generálva; CLI referencia teljes.
- [ ] Migrációs útmutató visszaállási úttal (a Deluge state érintetlen marad).
- [ ] Release notes felsorolja a breaking change-eket.
- [ ] `v1.0.0` tag + aláírt artifactok.

**Files:** `README.md`, `docs/`, `CHANGELOG.md`

---

## Első lépés

Ha most indul a kódolás: **M0-T1 → M0-T2 → M0-T3 → M0-T4**, és csak a harness eredménye után döntsünk a `libtorrent` ágról (ADR 0001) és a perzisztenciáról (ADR 0002).
