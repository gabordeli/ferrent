# Ferrent – roadmap

> Állapot: 2026-11-20, a port terve rögzítve (M4–M7); a kód az M0–M2-ben él, az M4-T1 első szelete elkezdve.
> Architektúra: [ARCHITECTURE.md](ARCHITECTURE.md).
> Sorrend: **MVP (v0.1)** → **v0.2–v0.5** (Deluge-plugin port) → **v1.0**. A taskok nem időbecslések, hanem lezárt, ellenőrizhető egységek.

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
- [ ] Az eredmények `docs/measurements/mvp.md`-ben rögzítve.
- [ ] Éles torrenteken (≥5 torrent, ~1 hét) bizonyított működés után `v0.1.0` tag (M2-T5).

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

- [ ] `crates/protocol/proto/ferrent/v1/ferrent.proto` verziózott; mezőszám nem újrahasznált; breaking-change ellenőrzés CI-ban.
- [ ] Kliens TLS-en át tud add/list/pause/resume/remove műveletet.
- [ ] Delta-stream: inaktív torrent 60 s alatt 0 üzenet; aktív ~1 Hz.
- [ ] Rossz token → `UNAUTHENTICATED`; pin-elt ujjlenyomat eltérés → egyértelmű hiba.
- [ ] `docs/adr/0004-transport.md`: gRPC vs WebSocket döntés indoklással.

**Files:** `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/protocol/src/client.rs`, `crates/daemon/src/rpc/`, `crates/daemon/tests/rpc_tls.rs`, `buf.yaml`, `docs/adr/0004-transport.md`

### M1-T4 – `ferrent-cli` 🤖

**Scope:** config profilok, add/list/show/pause/resume/remove, `watch` delta mód, JSON kimenet.

**Acceptance**

- [ ] Parancsok TLS-en, tokennel (config/env) működnek.
- [ ] `--json` kimenet dokumentált és stabil (mezőnevek verziózva).
- [ ] `ferrent-cli watch` streamel; Ctrl-C tisztán leáll.
- [ ] Integrációs teszt in-process daemon ellen.
- [ ] Súgó és exit code-ok dokumentálva.

**Files:** `crates/cli/src/main.rs`, `crates/cli/src/config.rs`, `crates/cli/src/output.rs`, `crates/cli/src/error.rs`, `crates/cli/tests/cli.rs`, `crates/daemon/src/main.rs`, `docs/cli.md`

### M1-T5 – Store + fastresume daemon integration

**Scope:** A `StateStore` rekordok (metainfo, mentési út, fastresume) bekötése a daemonba: add/remove perzisztálás, induláskori restore, SIGTERM/SIGINT fastresume-mentés.

**Acceptance**

- [ ] `ferrentd` add/remove után a `state.redb` tartalmazza/törli a torrent rekordot.
- [ ] Induláskor a daemon a rekordokat restore-olja, azonos ID-val; a fastresume átadódik az engine-nek.
- [ ] SIGTERM/SIGINT esetén a fastresume mentődik; `kill -9` esetén az utolsó mentés elavulhat (dokumentált).
- [ ] Valódi libtorrent resume roundtrip teszt: add → save_resume_data → új engine → restore, progress 1.0.
- [ ] Mock daemon restart: add → SIGTERM → restart → a torrent megjelenik; remove → restart → nincs.

**Files:** `crates/engine/src/types.rs`, `crates/engine/src/engine.rs`, `crates/engine/src/mock.rs`, `crates/engine/src/libtorrent/`, `crates/daemon/src/persist.rs`, `crates/daemon/src/store/memory.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/daemon/src/main.rs`

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

**Files:** `crates/gui/spikes/`, `docs/adr/0003-gui-framework.md`, `docs/measurements/m2-gui-spike.md`

### M2-T2 – GUI shell, kapcsolat, TLS pinning

**Scope:** app shell, szerverprofilok, TOFU fingerprint dialógus, újracsatlakozás, státuszsáv.

**Acceptance**

- [ ] Első csatlakozáskor mutatja az ujjlenyomatot; elfogadás után tárolja; eltéréskor figyelmeztet.
- [ ] Daemon-restart után az app újraindítás nélkül újracsatlakozik.
- [ ] A token biztonságosan tárolva (keyring vagy 0600 config), nem kerül logba.
- [ ] Hibák a UI-ban jelennek meg, nem panic.

**Files:** `crates/gui/src/main.rs`, `crates/gui/src/config.rs`, `crates/gui/src/connection.rs`, `crates/protocol/src/tls.rs`, `docs/measurements/m2-gui-shell.md`

### M2-T3 – Virtualizált torrent tábla + műveletek

**Scope:** oszlopok, server-side szűrés/rendezés, többszörös kijelölés, kontextusmenü, részletek panel.

**Acceptance**

- [ ] 4 000 sor görgetése ≥ 60 FPS; csak a viewport sorai kérdezhetők le.
- [ ] Rendezés/szűrés szerveroldali kérést indít; nincs teljes lista letöltés.
- [ ] `.torrent` hozzáadás nyers bájtokkal megy (távoli daemonon is működik).
- [ ] Pause/resume/remove optimista UI + hiba esetén visszaállítás.
- [ ] Gyakori műveletek billentyűparancsról; dokumentálva.

**Files:** `crates/gui/src/table.rs`, `crates/gui/src/actions.rs`, `crates/gui/src/main.rs`, `crates/gui/src/connection.rs`, `crates/gui/examples/demo_server.rs`, `docs/gui.md`, `docs/measurements/m2-gui-table.md`

### M2-T4 – MVP end-to-end acceptance

**Scope:** Debian + 4 000 torrent + GUI + CLI; mérés, soak, dokumentálás. A `v0.1.0` tag nem itt, hanem az éles használat után (M2-T5).

**Acceptance**

- [ ] R1/R2/R3 mérve és rögzítve (`docs/measurements/mvp.md`).
- [ ] 24 órás soak: nincs crash; RSS-növekedés < 10%.
- [ ] Restart recheck nélkül; indulás ≤ 30 s.
- [ ] GUI másik gépről TLS-en csatlakozik.
- [ ] Ismert korlátok dokumentálva.

**Files:** `docs/measurements/mvp.md`, `README.md`, `crates/daemon/examples/mvp_prepare.rs`, `crates/daemon/examples/rpc_bench.rs`, `ci/soak.sh`

### M2-T5 – v0.1.0 kiadás éles torrenteken

**Scope:** a kiadás kapuja nem a soak, hanem a valódi használat: a daemon legalább pár éles torrenttel fut a valódi szerveren, majd `v0.1.0` tag + release notes.

**Acceptance**

- [ ] Legalább 5 éles torrent legalább 7 napon át a daemonon, napi használat mellett.
- [ ] A torrentek tényleg dolgoznak: peer/tracker forgalom látszik, nem csak a state.
- [ ] Nincs crash és nincs kényszerű újraindítás; a Bugsinkben nincs új daemon-hiba.
- [ ] Éles restart recheck nélkül, a state (seeding/paused) megmarad; indulás ≤ 30 s.
- [ ] Tapasztalatok a `docs/measurements/mvp.md`-ben (éles szakasz); `v0.1.0` tag.

**Files:** `docs/measurements/mvp.md`, `README.md`

### M2-T6 – GUI: Figma logó integrálása

**Scope:** a sidebar szöveges „Ferrent” wordmarkje helyett a Figma MCP-vel exportált logó, becsomagolt assetként (mint a fontok).

**Acceptance**

- [ ] A logó a Figma MCP-vel exportálva (SVG vagy 2× PNG), a `crates/gui/assets/` alatt verziózva.
- [ ] A sidebar tetején a logó; sötét háttéren olvasható, 1440×900-on nem lóg ki, nem torzul.
- [ ] Az ablakikon (ahol a platform támogatja) szintén a logót használja.
- [ ] `cargo build -p ferrent-gui` zöld; screenshot a változásról.

**Files:** `crates/gui/src/main.rs`, `crates/gui/assets/`, `crates/gui/Cargo.toml`

### M2-T7 – GUI: Activity fül valós eseményekkel

**Scope:** a Details → Activity fül a polling-diffből érzékelt állapotváltásokat mutatja (checking → downloading → seeding, pause/resume, error); daemon oldali eseménytörténet egyelőre nem kell.

**Acceptance**

- [ ] A kliens a két poll közötti állapotkülönbségből eseményt képez (`checking→seeding`, `→error`, pause/resume, remove).
- [ ] Az esemény: időpont, torrent név/ID, előző→új állapot, forrás (kliens művelet vagy daemon-oldali váltás).
- [ ] Csak az utolsó N (pl. 200) esemény marad meg; a kiválasztott torrentre szűrhető.
- [ ] Kiválasztás nélkül összesített nézet vagy egyértelmű üres állapot.
- [ ] Legalább egy unit teszt a diff→esemény logikára.

**Files:** `crates/gui/src/main.rs`, `crates/gui/src/activity.rs` (új), `crates/gui/src/connection.rs`

### M2-T8 – Soak újrafuttatás a jelenlegi binárison

**Scope:** a most futó soak a redesign előtti binárison megy (a GUI seeds/peers értékei 0-k); a #25 után a jelenlegi `main` binárison kell megismételni.

**Acceptance**

- [ ] A jelenlegi binárison fut a 10 000 torrentes soak, ugyanazzal a `ci/soak.sh` mintavevővel.
- [ ] A GUI a soak daemonra csatlakozva valós seeds/peers/ETA értékeket mutat.
- [ ] A baseline (restore idő, RSS) és a CSV útvonala rögzítve; a 24h kiértékelés ehhez a futáshoz kötve.
- [ ] A futás a #25 kapuit használja; nincs párhuzamos soak a régi binárison.

**Files:** `ci/soak.sh`, `docs/measurements/mvp.md`, `crates/daemon/examples/mvp_prepare.rs`

---

# v0.2–v0.5 – Deluge-plugin port (M4–M7)

> A hat Deluge-funkció (AutoAdd, Label, Execute, Scheduler, DiskGuard, MobileUi)
> nem külön plugin, hanem a daemon beépített képességei. Részletes elemzés,
> döntések és kockázatok: [docs/plugin-port-analysis.md](docs/plugin-port-analysis.md).
> A kockázat/döntés jegyzet GitLabon is külön issue.
>
> GitLab: M4–M7 milestone-ok (#8–#11), task issue-k #30–#47, kockázat/döntés #48.

**Öt közös alap**

1. **Config service** – tipizált, validált, revíziózott `config.toml` + redb runtime state; `GetConfig`/`SetConfig`, atomikus apply, hot reload.
2. **Pause authority** – `USER`/`SCHEDULER`/`DISK_GUARD`/`RATIO` reasonök; effektív paused = bármelyik aktív; restartnál a `USER` marad, a tranziens újraszámolódik.
3. **Effektív beállítás-feloldás** – torrent override > label policy > session; a label nem mutálja a torrentet.
4. **Esemény-journal** – perzisztált domain események + kurzor; erre épül a hook, a notifications outbox, az Activity és a web SSE.
5. **Infohash** – first-class azonosító; duplicate add → `AlreadyExists{id}`; ez az AutoAdd-dedupe, az import és a cross-seed alapja.

**Rögzített döntések (2026-11-20):** egy TOML szekciókkal; label effektív feloldás, egy label/torrent (v1); scheduler piros = mindent pause-ol (`pause_seeding` opció); külön `ferrent-web` binary; DiskGuard add gate default `notify` (AutoAdd `defer`), `reject` opció; hook default argv, shell opt-in, 60 s timeout, max 4 párhuzamos.

---

## M4 – v0.2 Alapok: config, motor, pause, label

**Done when:** a daemon tipizált configból él és hot-reloadol; a GUI Settings modal fülekkel szerkeszt; a torrentnek van labelje és infohashje; a per-torrent limitek, a move storage és a `Finished` esemény működik; a pause reason modell tesztelt.

**Non-goals:** watch dir, scheduler, disk guard, hookok, notifications, web.

**Függ:** M1/M2.

### M4-T1 – Config service és séma 🤖

**Scope:** rétegezett config (file + env), szekciók, validáció, revision, atomikus apply, `GetConfig`/`SetConfig`, CLI `config get/set`; a redb a futásidejű label/watch állapoté (M4-T4/M5).

**Acceptance**

- [x] ismeretlen kulcs/érték → `apply_result.error`, nem csendes default.
- [x] elavult revision → `ABORTED`, a futó config nem változik.
- [x] érvényes `SetConfig` után reload nélkül él a változás + `ConfigChanged` event.
- [x] a token redacted; partial apply rollback tesztelt; 4 000 torrent mellett < 50 ms.
- [x] `ferrent-cli config get --json` és `set` működik a daemon ellen.

**Files:** `crates/daemon/src/config/{mod,schema,apply}.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/cli/src/main.rs`, `docs/adr/0006-config-service.md`

### M4-T2 – Motor-képességek 🤖

**Scope:** per-torrent limitek, session/queue limitek, `move_storage`, `Moving` state, infohash, `Finished`, duplicate add.

**Acceptance**

- [x] `TorrentEngine`: `set_torrent_limits`, `move_storage`, bulk pause/resume; `EngineSettings` + `active_*`.
- [x] `TorrentStatus.infohash`, `TorrentState::Moving` (proto 7); minden struct-literal builder fordul.
- [x] `EngineEvent::Finished` pontosan egyszer, restart után sem ismétlődik.
- [x] duplicate add → `AlreadyExists{id}`; a daemon a meglévő ID-t adja vissza.
- [x] valós libtorrent: azonos FS-en move recheck nélkül; cross-FS `Moving` → `Seeding` (a teszt `/dev/shm` célra mozgat).

**Files:** `crates/engine/src/{types,engine,mock,libtorrent}/*`, `crates/daemon/src/state.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`

### M4-T3 – Pause authority ⚠️

**Scope:** reason-modell, effektív állapot, perzisztencia, bulk session pause, UI reason.

**Acceptance**

- [x] property teszt: effektív paused = bármely reason aktív.
- [x] kézi pause → scheduler/guard `ok` nem indítja újra; kézi resume → guard `low` újra pause-ol.
- [x] restart után a `USER` megmarad, a tranziens reasonök újraszámolódnak.
- [x] 10 000 torrenten a bulk váltás < 1 s; csak a korábban futókat resume-olja.
- [x] `Torrent.pause_reasons` (uint32) a protóban; a GUI General fülön „Paused by” sor.

**Files:** `crates/daemon/src/control.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/daemon/src/persist.rs`, `crates/daemon/src/main.rs`, `crates/gui/src/main.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `docs/adr/0007-pause-authority.md`

### M4-T4 – Labels core 🤖

**Scope:** label CRUD + policy (move target, limitek, auto-match predicate), hozzárendelés, effektív feloldás, szűrés/rendezés, move-on-complete.

**Acceptance**

- [x] egy torrent egy label (v1); label törlése → No Label.
- [x] effektív limit = torrent override > label > session; levételkor a kézi override megmarad.
- [x] `ListTorrents` label szűrő/rendezés; 4 000 torrent p95 < 50 ms (mért: `label_query_p95...` teszt).
- [x] `Finished` → `move_completed_path`; siker/hiba event; a DiskGuard látja a célt.
- [x] auto-match: tracker substring + név regex; új torrentnél és label létrehozásakor is lefut.

**Files:** `crates/daemon/src/labels.rs`, `crates/daemon/src/persist.rs`, `crates/daemon/src/state.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `docs/adr/0008-labels.md`, `tests/`

### M4-T5 – GUI Settings workbench + label UI 🤖

**Scope:** 900×640 modal, bal fülsáv (Connection, Session, Labels), tábla + egy Save + Reload + dirty, validáció; label oszlop/szűrő/context menü.

**Acceptance**

- [x] a Settings modal fülekkel nyílik, Esc/Close, 1440×900-on elfér.
- [x] egy Save az összes tabot menti; Reload eldobja; dirty jelzés; hiba a sor mellett.
- [x] Label tábla: név, szín, move path, limitek, auto-match; egy Save.
- [x] táblában label oszlop, sidebar label szűrő, hozzárendelés context menüből (`Set label…` → modal).
- [x] screenshot + X11 smoke + `cargo test -p ferrent-gui`.

**Files:** `crates/gui/src/settings.rs`, `crates/gui/src/connection.rs`, `crates/gui/src/table.rs`, `crates/gui/src/main.rs`, `crates/gui/src/icons.rs`, `crates/gui/tests/settings.rs`

---

## M5 – v0.3 Automatizálás: AutoAdd, hookok, scheduler, notifications

**Done when:** watch dir → add → label → limit → move → finished hook end-to-end működik; a scheduler három szintje restart- és DST-biztos; a webhook és a Telegram kézbesítés retry-jal, dedupe-pal és digesttel megy; a GUI tabok készen vannak.

**Non-goals:** disk guard, web, ratio-alapú auto-remove (M8), email/ntfy/push.

**Függ:** M4.

### M5-T1 – AutoAdd 🤖

**Scope:** watch dir CRUD, 1 s reconcile scan, infohash dedupe, post-add action, invalid retry, label/save path/add paused, free-space gate, GUI tab.

**Acceptance**

- [x] 100 `.torrent` → mind egyszer, p95 < 2 s. A scan 1 s-es poll + size/mtime stabilitás (inotify helyett); 100 fájl egy menetben < 2 s (`tests/autoadd.rs` méri).
- [x] duplikált fájl más néven → nincs második torrent; a fájl a configured action szerint mozog/törlődik.
- [x] hibás/0 bájtos → 3 kísérlet után `.invalid`; a számláló redb-ben, restart után megmarad (`RedbStore` újranyitás-teszt).
- [x] `.magnet` egy és több sorral; a label az adddal együtt kerül be (add_paused-dal együtt).
- [x] opcionális disk gate (`min_free_bytes`): kevés hely → a fájl a watch dirben marad, és nem fogyaszt kísérletet.

**Files:** `crates/daemon/src/autoadd.rs`, `crates/daemon/src/config/schema.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `crates/daemon/tests/autoadd.rs`

### M5-T2 – Execute hookok ⚠️

**Scope:** esemény-journal + ack, hook entitás a configban, argv/env contract, timeout/concurrency, futásnapló, GUI tab.

**Acceptance**

- [x] added/finished/removed/error eseményre futtat; argv `{id} {name} {path} {infohash} {label}` (+ `FERRENT_*` env).
- [x] at-least-once: a nyugtázatlan `hookevent` rekord redb-ben marad és restart után újra lefut (ADR 0009; idempotencia a hooké).
- [x] timeout → process group `SIGKILL`; exit code/stdout/stderr/időtartam a `hookrun` rekordban, a GUI-ban is.
- [x] max 4 párhuzamos (teszt méri); a torrent-szál nem blokkol (tokio task + külön process); shell csak `shell = true`-ra.
- [x] `removed` esetén a név/path/infohash a remove előtti mirror-pillanatképből jön.

**Files:** `crates/daemon/src/events.rs`, `crates/daemon/src/hooks.rs`, `crates/daemon/src/config/schema.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `docs/adr/0009-event-journal.md`

### M5-T3 – Scheduler ⚠️

**Scope:** 24×7 rács (zöld/sárga/piros), időzóna, next-boundary tick, session + aktív limitek, `SCHEDULER` reason, GUI tab, státusz.

**Acceptance**

- [x] tick ≤ 1 s pontossággal a határnál; suspend után újraszámol (abszolút óra-határ, max 30 s-es újraértékelés).
- [x] zöld = session default; sárga = limitek; piros = scheduler pause, csak a futókat resume-olja (`set_many` → csak a tényleg változók).
- [x] kézi pause megmarad (a `USER` reason marad); a guard reason-t nem bántja.
- [x] DST tavasz/ősz teszt `chrono-tz`-vel (2026-03-29 és 2026-10-25, Budapest); státusz `GetStatus`-ban (`scheduler_enabled/level/next_unix`).
- [x] 10 000 torrenten a váltás < 1 s (integration teszt méri, pause és resume).

**Files:** `crates/daemon/src/scheduler.rs`, `crates/daemon/src/control.rs`, `crates/daemon/src/config/schema.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `crates/daemon/tests/scheduler.rs`

### M5-T4 – Notification router + webhook 🤖

**Scope:** esemény → szabály → outbox → csatorna; webhook JSON + HMAC; retry/backoff; dead-letter; secret-kezelés; GUI/CLI test.

**Acceptance**

- [x] az esemény a szabály alapján outbox rekordot kap; `(event_seq, url)` dedupe; restart után nincs dupla.
- [x] webhook: JSON POST, `X-Ferrent-Event`, `X-Ferrent-Delivery`, opcionális `X-Ferrent-Signature: sha256=…`; timeout (`timeout_ms`).
- [x] 5xx/429 → exponenciális backoff (5 s · 2^n, cap 300 s), max 5 kísérlet, utána `dead` (a statusban és a GUI-ban; Activity bejegyzés a későbbi feeddel jön).
- [x] a secret nem kerül logba; a webhook URL csak http/https (config-validáció + GUI-validáció).
- [x] Settings → Notifications tábla + egy Save/Reload + „Test”; `ferrent-cli notify test` / `notify status`.
- [x] fake HTTP szerverrel tesztelt (TcpListener, 200/400/500); nincs külső hálózat a tesztekben.

**Files:** `crates/daemon/src/notify.rs`, `crates/daemon/src/config/schema.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `crates/cli/src/main.rs`, `crates/daemon/tests/notify.rs`

### M5-T5 – Telegram + digest/quiet hours 🤖

**Scope:** Telegram Bot API csatorna, HTML escape, 4 096 karakter, 429 `retry_after`; digest/coalescing; quiet hours; label-szűrés.

**Acceptance**

- [x] `sendMessage` `chat_id`-dal és opcionális `message_thread_id`-vel; a hosszú szöveg 4 096 karakterre vágva (HTML-escape).
- [x] 429 esetén `retry_after` szerint vár (a backoff helyett); a token nem kerül logba.
- [x] digest: 10 esemény vagy 30 s ablak; 100 `finished` → 10 üzenet; 1 000 → 100 üzenet (< 60 s, teszt).
- [x] quiet hours (`HH:MM` + időzóna) + globális/per-csatorna label-szűrés; a „Test” minden engedélyezett csatornára küld.
- [x] mock HTTP szerveres tesztek (200/429 `retry_after`, HTML body, digest/quiet).

**Files:** `crates/daemon/src/notify.rs`, `crates/daemon/src/config/schema.rs`, `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `crates/daemon/tests/notify.rs`

### M5-T6 – Automatizálás acceptance 🔬

**Scope:** E2E + restart + config reload + mérés.

**Acceptance**

- [x] watch dir → add → label → limit → move → finished hook → webhook E2E; Telegram mockkal (fake HTTP szerver, `tests/automation_e2e.rs`).
- [x] restart közben nincs elveszett/duplázott add, hook vagy notification (két runtime-mal; journal + outbox + infohash dedupe).
- [x] config reload frissíti a futó szabályokat; hibás config → régi marad.
- [x] mérés: 100 fájl, 1 000 torrent; `docs/measurements/automation.md` (add pass 10,1–14,4 ms release).

**Files:** `crates/daemon/tests/automation_e2e.rs`, `docs/measurements/automation.md`; a move-path javítás: `crates/daemon/src/state.rs`, `crates/daemon/src/persist.rs`, `crates/daemon/src/rpc/mod.rs`

---

## M6 – v0.4 DiskGuard

**Done when:** lemezenkénti zónák (címkézett könyvtár + staging), automatikus letöltési út ugyanazon a lemezen (rename, nem cross-disk copy), guard, add-time kapu és GUI; több mountos hoston bizonyított.

**Non-goals:** auto-remove, SMART, nem-Ferrent pathok; preallocate az M6-T5-ben (később).

**Függ:** M4 (label), M4-T2 (`Moving`).

**Modell (2026-11-20, user-döntés):**

- **Zóna = mappa, nem lemez.** Két fajta: *library* (címkével, **1:1**) és *staging* (lemezenként **egy, kötelező**).
- Feloldás hozzáadáskor: `címke → library zóna → st_dev → azonos st_dev-en lévő staging → save_path`. A lemez futásidőben a pathból számolt `st_dev`, így az átmountolás nem töri el.
- Címke nélkül a globális `download_dir` marad; a hozzáadó dialog figyelmeztet.
- A letöltési út a dialogban látszik és **szerkeszthető**; címke választásra automatikusan frissül.
- A zóna az igazságforrás: a `Label.move_completed_path` megszűnik, a hely a zónában él.
- Befejezéskor a move a library zónába megy; a cél tele → a torrent a stagingen seedel és vár (retry + notification).

### M6-T1 – Storage + zónamodell 🤖

**Scope:** mount tábla, `statvfs`, `st_dev` csoportosítás/aliasok, `[[diskguard.zone]]` (library/staging), feloldás, unknown, payload.

**Acceptance**

- [x] `/proc/self/mounts` parse + pseudo FS szűrés; bind mountok egy disk (leghosszabb alias a display).
- [x] payload: mount, fs, total/free/used, zones, torrents, labels, low; unknown[].
- [x] zóna validáció: üres/relatív path, duplikált címke (kis-nagybetűtől függetlenül), ugyanaz a path két zónában, staging címkével, library címke nélkül.
- [x] `resolve_staging(label)` = címke → library zóna → `st_dev` → staging; ismeretlen címke / staging nélküli lemez → típusos hiba (`ZoneError`).
- [x] tmpfs + bind mount a mount-tábla fixture-ön; valódi path stat + eltűnt path → unknown, nem hiba.
- [x] `statvfs` + teljes scan `spawn_blocking` + 5 s timeout; hiba/timeout → nincs payload (a hívó az utolsó készletet tartja).
- [x] 1 000 zóna csoportosítása < 50 ms (teszt); `snapshot` a valódi fájlrendszeren.

**Files:** `crates/daemon/src/diskguard/storage.rs`, `crates/daemon/src/diskguard/zones.rs`, `crates/daemon/src/diskguard/mod.rs`, `crates/daemon/src/config/schema.rs`, `crates/daemon/tests/diskguard.rs` (a proto/RPC a T4-ben)

### M6-T2 – Automatikus letöltési út + move 🤖

**Scope:** add-kori staging-feloldás, `save_path` felülírás, move a zónába, teljes cél-lemez viselkedés, add gate.

**Acceptance**

- [x] `AddTorrent` üres `save_path` + címke → a zóna lemezének stagingje; explicit `save_path` nyer (új proto mező: `label`).
- [x] AutoAdd: a watch dir `save_path` opcionális; üresen a címke szerinti staging (ha a címkének még nincs zónája, a session `download_dir` marad).
- [x] befejezéskor a move a library zónába megy; azonos `st_dev` → nincs szóköz-kapu, tehát rename (a valódi rename mérés a T4 hoston, a mocknak nincs payloadja).
- [x] cél-lemez tele a move idején → Seeding a stagingen, retry 5× / 300 s + `Error` event; utána feladja.
- [x] címkeváltás élő torrenten nem mozgat; a következő move már a zónát használja.
- [x] add gate: staging FS < `min_free` → `FAILED_PRECONDITION` (`on_low = "reject"`; `"notify"` enged + `Error` eventet ír); AutoAdd defer, nem `.invalid`.

**Files:** `crates/daemon/src/diskguard/{paths,moves}.rs`, `crates/daemon/src/autoadd.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `crates/daemon/tests/{diskguard,autoadd,automation_e2e}.rs`

### M6-T3 – Guard engine ⚠️

**Scope:** threshold/hysteresis, notify/pause, csak aktív írók, `DISK_GUARD` reason, `DiskLow`/`DiskOk`.

**Acceptance**

- [x] low = `free < min`; ok = `free >= min + margin`; csak tranziens emitál.
- [x] pause csak Downloading/Queued/Checking/Moving; seed nem; kézi pause-t nem ír felül (közös reason-bitset).
- [x] `DiskLow`/`DiskOk` esemény a M5 csatornákra (a hook/notification szerkesztő is ismeri a két nevet).
- [x] restart után újraértékel (üres szint-térkép → első passban tranzíció); hiba/timeout → az utolsó payload marad (`storage_payload`).
- [x] két fájlrendszer (build FS + tmpfs) szimuláció: csak az alacsony lemez írói pause-olódnak, a másiké nem; a 3 FS éles ellenőrzés a T4 hoston.

**Files:** `crates/daemon/src/diskguard/guard.rs`, `crates/daemon/src/control.rs`, `crates/daemon/src/events.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/gui/src/settings.rs`, `crates/daemon/tests/diskguard.rs`

### M6-T4 – API + GUI + acceptance 🤖 🔬

**Scope:** `GetDisks` + zóna API, Settings → Storage (lemezek + zónák), add-torrent dialog, sidebar, multi-mount mérés.

**Acceptance**

- [x] `GetDisks`: mount, fs, total/free/used, low, zónák (path, label|staging, enabled), torrentszám; a zóna nélküli valódi mountok is (statvfs), `unknown[]`; CLI `ferrent disks [--json]`, `ferrent add --label`.
- [ ] Settings → Storage: lemez-tábla (total/free/min/margin) + zóna-lista hozzáadás/törlés, **egy** Save + explicit Reload.
- [ ] Add-torrent dialog: letöltési út mező, címke választásra frissül, szerkeszthető; címke nélkül warning.
- [ ] sidebar „N disks · X free”, low → piros, kattintás a Storage tabra.
- [ ] 3 FS-es host + tmpfs low szimuláció; mérés: poll idő, pause/resume latency, 4 000 torrent; `docs/measurements/diskguard.md`.

**Files:** `crates/gui/src/settings.rs`, `crates/gui/src/main.rs`, `crates/protocol/proto/ferrent/v1/ferrent.proto`, `docs/measurements/diskguard.md`

### M6-T5 – Preallocate-kapu (később) 🤖

**Scope:** opcionális preallocate: a torrent teljes méretének ellenőrzése a hozzáadáskor.

**Acceptance**

- [x] `[diskguard] require_space_for_torrent = true` (default `false`), GUI checkbox a Storage fülön.
- [x] bekapcsolva: `free < total_bytes + margin` → add `FAILED_PRECONDITION`, érthető üzenettel (lemez, kell / van); AutoAdd defer, nem `.invalid`. A méret a metainfóból jön, ha a kliens nem küldi.
- [x] kikapcsolva a mai viselkedés (csak `min_free` gate).

**Files:** `crates/daemon/src/diskguard/`, `crates/daemon/src/rpc/mod.rs`, `tests/`

---

## M7 – v0.5 WebUI

**Done when:** `ferrent-web` + SPA mobilról használható; lista/detail/add/label/pause; SSE; 10 000 torrent.

**Non-goals:** settings szerkesztés, push (v1.1), teljes parity.

**Függ:** M4 (label), M5/M6 státusz opcionális.

### M7-T1 – `ferrent-web` skeleton + auth + SSE ⚠️

**Scope:** axum, TLS/reverse proxy, token/password, session, statikus SPA, `/health`, SSE gateway 1 Hz coalescing.

**Acceptance**

- [x] csatlakozik a daemonhoz TLS pinninggel (CA + opcionális SHA-256 fingerprint); a tokent nem logolja.
- [x] login → HttpOnly session cookie; read-only mód; 5/perc limit; logout.
- [x] SSE: csak változott torrentek, coalesce ablak (alap 1 s), 15 s keep-alive; a reconnect utáni read-szinkron a T2 API-val jön.
- [x] `/health` és a placeholder SPA azonos originon; nincs CORS réteg.
- [x] ADR 0010: külön binary (nginx terminálja a TLS-t).

**Files:** `crates/web/src/{main,config,auth,sse}.rs`, `docs/adr/0010-web-gateway.md`

### M7-T2 – REST API (thin) 🤖

**Scope:** list/detail/files/add/pause/resume/remove/label/bulk; szerveroldali filter/sort/page; add magnet + file + paused + label.

**Acceptance**

- [x] a végpontok a Ferrent RPC-t tükrözik; duplicate → 409 + id; magnet **és** base64 metainfo add (a proto `magnet` mezővel).
- [x] lista offset/limit/filter/state/label/sort; a 10k p95 mérés a T4-ben (a daemon oldali p95 48 ms 4 000-en).
- [x] hibakódok 400/401/403/404/409/412 (+429/502/503); `docs/web-api.md`.
- [x] bulk max 1 000 id, részleges `ok`/`failed`.

**Files:** `crates/web/src/api.rs`, `docs/web-api.md`, `tests/`

### M7-T3 – SPA port a webui2-ből 🤖

**Scope:** Vue app adapter (`api.ts`, `types.ts`), lista/detail/add/label, mobil gesztusok, i18n/theme, PWA.

**Acceptance**

- [x] a meglévő SPA a Ferrent API-val fut: `api.ts` adapter (login, lista, detail, add magnet/fájl, pause/resume/remove, label, bulk, SSE `deltas`); a Deluge-only panelek (queue, per-file priority, push, dirs) default értékkel.
- [x] 10k torrent: a lista 10 DOM-sort tart (`10001 torrent 50 betöltve`), 360/390 px-en nincs vízszintes scroll (Playwright méri).
- [x] HU/EN, light/dark, offline banner (`navigator.onLine`), PWA manifest/SW átnevezve Ferrentre.
- [x] Playwright smoke mock API ellen (3 teszt, 0 console error) + vitest 150 zöld; `webui/dist` kiszolgálása a gatewayből (`spa_dir`, SPA-fallback).

**Files:** `webui/src/api.ts`, `webui/src/types.ts`, `webui/src/App.vue`, `webui/e2e/`

### M7-T4 – Deployment + acceptance 🔬

**Scope:** systemd/nginx, read-only dashboard, telefonos teszt, perf.

**Acceptance**

- [x] **LAN-mód**: a gateway maga terminálja a HTTPS-t (saját, first startkor generált cert, SAN a hostnév + LAN-IP, `Secure` cookie); nincs reverse proxy; a daemon port loopback. A publikus (443/port forward) későbbi döntés.
- [x] telefonos méretben (Playwright 390×844 + 360 px): list → detail → pause → add → label; a fizikai telefonos kör a user oldalán.
- [~] 10k torrent, 3 SSE kliens, 1 óra: az RSS-mérés fut (`docs/measurements/webui.md` M4, CSV); az első minta 15,8 MB, a kapu < 150 MB.
- [x] mérés + telepítési doksi (`docs/measurements/webui.md`, `docs/web-install.md`); `read_only` config; systemd unit (`packaging/systemd/ferrent-web.service`) + `web.toml` minta.

**Files:** `packaging/systemd/ferrent-web.service`, `docs/web-install.md`, `docs/measurements/webui.md`

---

# v1.0 – telepíthető, migrálható, üzemeltethető

**Definíció:** `apt install ferrentd` egy tiszta Debianon; a meglévő Deluge-állomány átemelhető; a Deluge-pluginok funkciói beépítve (labels, AutoAdd, scheduler, hooks, notifications, DiskGuard, web UI); a *arr stack és a VPN-es homelab használat kényelmes; a rendszer soak-tesztelt.

**Done when**

- [ ] Aláírt APT repo + `systemctl` + Docker image; friss telepítés dokumentált.
- [ ] `import-deluge` 4 000 torrentet visz át, újraellenőrzés nélkül (vagy dokumentált fallbackkel).
- [ ] A v0.2–v0.5 funkciók élesben: labels + AutoAdd + scheduler + hooks + notifications + DiskGuard + web UI.
- [ ] Sonarr/Radarr add + list működik az emulált qBittorrent API-n.
- [ ] VPN-interfész kötés leak-teszttel bizonyítva; `/metrics` Grafana-kompatibilis.
- [ ] 72 órás soak + `kill -9` recovery zöld; nincs fd/RSS szivárgás.
- [ ] Dokumentáció (telepítés, CLI, API, migráció) és `v1.0.0` kiadás.

**Non-goals (v1.0)**

- Natív mobil kliens, média-streaming, több daemon klaszter.
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

- [x] `cargo deb -p ferrent-daemon` előállítja a binárist, unitot, configot, man page-et.
- [x] Friss Debian 12-n telepítés után `systemctl start ferrentd` működik; a `ferrent` user létrejön.
- [x] `apt remove` megtartja a `/var/lib/ferrent`-et; `purge` törli (dokumentálva).
- [x] A csomagban nincs secret; a config jogosultsága 0640, tulajdonos `root:ferrent`.
- [x] Ismételhető ellenőrzés: `packaging/debian/container-test.sh` (31 assert, Debian 12 és 13, `docs/measurements/packaging.md`). A `$auto` miatt a `.deb`-et a cél-suite-ban kell építeni (bookworm recipe a `packaging/debian/README.md`-ben; a build konténer a T2).

**Files:** `packaging/debian/`, `crates/daemon/Cargo.toml` (`[package.metadata.deb]`)

### M3-T2 – Statikus `libtorrent` build + több disztrós `.deb` ⚠️

**Scope:** statikus link, build konténer, CI mátrix (Debian 12/13, Ubuntu 24.04), arm64.

**Acceptance**

- [x] `ldd ferrentd` nem mutat `libtorrent` függőséget; csak a vállalt glibc/libstdc++ baseline.
- [x] Ugyanaz a `.deb` települ és fut Debian 12, 13 és Ubuntu 24.04 konténerben (32/32 assert mindhármon).
- [x] A bináris méret dokumentálva (26,8 MB amd64 / 18,2 MB arm64; `docs/measurements/packaging.md`); a build a `packaging/debian/build.sh` konténerben, rögzített libtorrent verzióval.
- [x] arm64 build (cross `aarch64-linux-gnu`, `qemu-smoke.sh` alatt fut) lefut és smoke-tesztet kap.
- [x] `docs/adr/0005-min-debian.md`: minimum Debian verzió döntés (Debian 12+, Ubuntu 24.04+).

**Files:** `packaging/debian/build.sh`, `.github/workflows/release-deb.yml`, `docs/adr/0005-min-debian.md`

### M3-T3 – APT repo pipeline

**Scope:** tag → build → GPG-sign → index → GitHub Pages; DEB822 `Signed-By`; régi verziók megtartása.

**Acceptance**

- [~] Tag push után `apt update && apt install ferrentd` működik tiszta Debianon — lokálisan igazolva (`repo-test.sh`, 6/6 PASS, DEB822 + `Signed-By`, `file:` forrás); a tag→`gh-pages` út a workflow-ban kész, valódi GitHub repó + Pages beállítás kell hozzá.
- [x] A kulcs dokumentált (`packaging/apt/README.md`, `docs/apt-install.md`); `Signed-By=/etc/apt/keyrings/ferrent.gpg`; nincs `apt-key`.
- [x] Az index checksumokat tartalmaz (MD5/SHA1/SHA256/SHA512) és a saját aláírását `gpgv` ellenőrzi; az előző verzió a poolban marad (a `repo-test.sh` 0.0.9-1-ről 0.1.0-1-re frissít).
- [x] Hiányzó `GPG_PRIVATE_KEY` → explicit `::error::` és bukó job a publish lépésben.

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

## M8 – Ökoszisztéma (v1.0)

**Done when:** a Deluge-ból át lehet költözni; a *arr stack és a VPN-es homelab kiszolgálható; az arány/idő alapú auto-remove kész (a labels/hooks/scheduler a v0.2–v0.3-ban).

**Non-goals:** teljes Deluge RPC emuláció, plugin ABI, torrent kereső/indexer.

**Függ:** M3, M7

### M8-T1 – `import-deluge`

**Scope:** Deluge `state` könyvtár beolvasása (`.torrent` + `.fastresume` + címkék) + a plugin-configok (`label.conf`, `autoadd.conf`, `execute.conf`, `scheduler.conf`) mappingje; dry-run, idempotens import.

**Acceptance**

- [ ] `ferrent-cli import-deluge --state-dir ... --dry-run` listázza a talált torrenteket és a felismert plugin-configokat.
- [ ] 4 000 torrent importja lefut; a címkék megőrződnek; a duplikátumok (infohash) kimaradnak.
- [ ] A `label.conf` → label-hozzárendelés, `autoadd.conf` → watch dir, `execute.conf` → hook, `scheduler.conf` → rács; a host-függő pathokról figyelmeztet.
- [ ] Kompatibilis fastresume esetén nincs recheck; ellenkező esetben dokumentált fallback.
- [ ] Újrafuttatás nem duplikál (idempotens); kis valós Deluge-fixture-rel tesztelve.

**Files:** `crates/cli/src/import_deluge.rs`, `tests/fixtures/deluge-state/`

### M8-T2 – qBittorrent Web API emuláció (*arr)

**Scope:** auth/login, `torrents/add`, `torrents/info`, kategóriák, pause/resume/delete; opt-in végpont.

**Acceptance**

- [ ] Sonarr/Radarr hozzáad egy torrentet és látja a queue-ban az emulált endpointon.
- [ ] Az API-verzió endpoint válaszol; cookie/header auth működik.
- [ ] Kategória → mentési útvonal mapping.
- [ ] Szerződés-tesztek rögzített kérésekkel.
- [ ] Feature flag mögött; dokumentálva.

**Files:** `crates/daemon/src/compat/qbittorrent.rs`, `crates/daemon/tests/compat/`

### M8-T3 – Arány/idő alapú auto-remove

**Scope:** a label policy + session default `stop_at_ratio`/`remove_at_ratio`/seed idő használata; a pause authority `RATIO` reasonje; dry-run; törlés adattal vagy anélkül.

**Acceptance**

- [ ] a limit a label policy-ból vagy a session defaultból jön; a kézi override érvényesül.
- [ ] dry-run listázza a jelölteket; élesben csak a limitet elért torrentek törlődnek.
- [ ] `Moving`/`Checking` alatt nincs törlés; a `RATIO` reason a pause authorityn megy.
- [ ] esemény + notification a törlésről; a szabály-kiértékelés unit tesztelt.
- [ ] hibás/elérhetetlen path → a torrent nem törlődik, hanem `dead` jelölést kap.

**Files:** `crates/daemon/src/automation/ratio.rs`, `crates/daemon/src/control.rs`, `crates/daemon/tests/`

### M8-T4 – VPN interfész-kötés + Prometheus `/metrics`

**Scope:** `outgoing_interfaces`/`listen_interfaces` kötés, link-figyelés, szüneteltetés interfész-vesztéskor; opcionális metrics endpoint.

**Acceptance**

- [ ] `wg0`-hoz kötve, az interfész leállításakor nincs kimenő forgalom (`ss`/`tcpdump` bizonyíték).
- [ ] Visszatéréskor automatikus resume, konfigurálható grace period.
- [ ] `/metrics`: torrentszám, le/fel sebesség, peer-ek, fd, RSS; Grafana scrape működik.
- [ ] A metrics endpoint opcionális és auth-konfigurálható.

**Files:** `crates/daemon/src/netbind.rs`, `crates/daemon/src/metrics.rs`

---

## M9 – Hardening és 1.0 kiadás

**Done when:** a rendszer soak- és crash-tesztelt, a biztonsági és teljesítmény-regresszió kapuzott, a dokumentáció teljes, a `v1.0.0` kiadva.

**Non-goals:** új nagy funkció; csak hardening, mérés, dokumentáció, release.

**Függ:** M8

### M9-T1 – Soak, crash recovery, robustness

**Scope:** 72 órás soak 4 000 torrenttel; `kill -9` recovery; sérült DB rekord; malformed protokoll üzenet.

**Acceptance**

- [ ] 72 h soak: nincs crash; RSS-növekedés < 10%; nincs fd-szivárgás.
- [ ] `kill -9` után restart recheck nélkül; az állapot konzisztens.
- [ ] Sérült DB rekord → a daemon indul, a rossz rekordot karanténba teszi.
- [ ] Malformed RPC üzenet → hiba, nem panic.
- [ ] Eredmények `docs/measurements/v1-soak.md`-ben.

**Files:** `tests/soak/`, `docs/measurements/v1-soak.md`

### M9-T2 – Biztonság + teljesítmény-regresszió

**Scope:** auth/TLS review, függőség-audit, nightly perf harness a büdzséhez képest; threat model.

**Acceptance**

- [ ] Nincs secret a logokban; a token rotáció működik.
- [ ] `cargo audit` tiszta vagy dokumentált kivételekkel.
- [ ] A perf harness baseline-hoz hasonlít; >10% regresszió esetén a nightly fail.
- [ ] TLS konfig és pinning tesztelve; rövid threat model doksi.
- [ ] `docs/security.md` kitöltve.

**Files:** `.github/workflows/perf.yml`, `docs/security.md`

### M9-T3 – Dokumentáció + `v1.0.0` release

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

A plugin port kritikus útja: **M4-T1 → M4-T2 → M4-T3 → M4-T4 → M4-T5**, majd **M5-T1 → M5-T6**. Az M6 az M4-T2/T4 után, az M7 az M4-T4 után indulhat; az M3 csomagolás párhuzamosítható. A részletes elemzés: [docs/plugin-port-analysis.md](docs/plugin-port-analysis.md).

**Állapot (2026-11-20):** M4, M5 és M6 teljes; **M7-T1–T4 kész** LAN-módban (gateway + auth + SSE + REST + SPA + HTTPS + unit/doksi; a publikus reverse proxy későbbi döntés). **M3-T1 kész** (`.deb` + unit + man page, 32 assertes konténer-teszt), **M3-T2** (statikus libtorrent, egy csomag Debian 12/13 + Ubuntu 24.04-ra, arm64 cross), **M3-T3** (aláírt APT repó + GitLab Debian Package Registry). **Verzió-lánc kész**: Cargo.toml az igazság, tag-ellenőrzés, `0.1.0-rc1` → `0.1.0~rc1-1`.
