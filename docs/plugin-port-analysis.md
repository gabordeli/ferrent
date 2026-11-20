# Ferrent – Deluge-plugin port elemzés

> Állapot: 2026-11-20, **terv**. A megvalósítás sorrendje: [ROADMAP.md](../ROADMAP.md)
> M4–M7. A döntések és kockázatok rögzítve; a GitLabon külön issue követi őket
> (#48).

## 1. Kiindulás: mit tud ma a Ferrent

| Terület | Ma | Forrás |
|---|---|---|
| Config | tipizált TOML szekciókkal, env-layering, revision, atomikus apply, hot reload | `crates/daemon/src/config/` |
| RPC | add/pause/resume/remove/get/list/status/files/peers/trackers/watch/queue-move/config get-set | `crates/protocol/proto/ferrent/v1/ferrent.proto` |
| State mirror | név-substring + state szűrő, 6 rendezési kulcs | `crates/daemon/src/state.rs` |
| Perzisztencia | név, save path, metainfo, fastresume, queue order | `crates/daemon/src/persist.rs` |
| Engine | per-torrent limitek, `move_storage`, `Moving`, infohash, `Finished`, session/active limitek | `crates/engine/src/{engine,types}.rs` |
| Események | `Added`, `StateChanged`, `Progress`, `Removed`, `Finished` (pontosan egyszer) | `crates/engine/src/types.rs` |
| GUI | virtualizált tábla, Details fülek, szerverkezelő; Settings modal 560×380 stub | `crates/gui/src/main.rs` |
| Label / infohash / move storage | label CRUD + assignment + policy; infohash; move storage | `crates/daemon/src/labels.rs`, `crates/engine/src/types.rs` |
| Web | nincs | – |

A Settings gomb és a modal már létezik, de üres: ez a port természetes control plane-je.

## 2. Deluge-pluginok

### 2.1 AutoAdd 1.8

- Watch dir-enként 5 s `LoopingCall`; `.torrent` és `.magnet`; több magnetet tartalmazó fájl feldarabolása.
- Hibás torrent: max 10 kísérlet, majd `.invalid`; a számláló memóriában van.
- Siker után: törlés, `.added` utótag, vagy másik mappába mozgatás; opcionális label, queue top/bottom.
- Dedupe: a Deluge „duplicate torrent” hibájára épül; nincs perzisztens látott-index.
- A label a sikeres add **után** kerül be, külön lépésben.

**Átvenni:** watch dir életciklus, több-magnet kezelés, készülő fájl türelmi logika.
**Kerülni:** memóriabeli dedupe, post-hoc label.

### 2.2 Label 0.3

- `label.conf`: `labels{id: options}` + `torrent_labels{torrent_id: label_id}`; egy torrent egy label.
- Opciók: limitek, queue/ratio, move completed, auto-add tracker substring.
- A hozzárendelés ráírja a torrent beállításait; levételkor a core defaultokra állít vissza.
- `STATE`/`KEYWORD` konstansok definiálva, de nem használtak.

**Átvenni:** label mint first-class fogalom, filter-tree.
**Kerülni:** a beállítások mutálását és a kézi override elvesztését.

### 2.3 Execute 1.3

- `execute.conf`: `(id, event, command)`; események: `added`, `complete`, `removed`.
- argv: `torrent_id`, `name`, `download_location`; `expandvars`/`expanduser`.
- Nincs timeout, retry, concurrency limit; a kimenet csak logba kerül.
- `removed` a `PreTorrentRemovedEvent` cache-ből dolgozik.

**Átvenni:** egyszerű argv-kontraktus, remove előtti pillanatkép.
**Kerülni:** fire-and-forget.

### 2.4 Scheduler 0.3

- `scheduler.conf`: `button_state[24][7]` (0 zöld, 1 sárga, 2 piros) + `low_*` limitek.
- Óránkénti `callLater` a következő óra határára; zöld = core defaultok, sárga = session limitek, piros = `pause_session()`.
- A session pause felülírhatja a kézi pause-t és a guard pause-t; suspend/DST mellett a timer elcsúszik.

**Átvenni:** háromszintű modell, 7×24 rács.
**Kerülni:** a session-pause-ot mint egyetlen mechanizmust.

### 2.5 DiskGuard (saját)

- `/proc/self/mounts` + `statvfs`; pseudo FS-ek kiszűrve; `st_dev` szerinti csoportosítás; leghosszabb mountpoint a display.
- PathUse: session `download_location`, torrent `save_path`/`download_location`, `move_completed_path` (session, torrent, label).
- 30 s poll, `deferToThread`, hiba esetén utolsó payload.
- Low = `free < min`; ok = `free >= min + margin` (hysteresis).
- `pause` csak az aktív írókat (`Allocating`, `Checking`, `Downloading`, `Moving`, `Queued`), a seedelőket nem; a guard saját pause-halmaza miatt a kézi pause-t nem indítja újra.
- Nincs add-time kapu, nincs auto-remove.

**Átvenni:** szinte mindent; **hozzáadni:** add-time és move-cél kapu.

### 2.6 MobileUi / webui2 (saját)

- Core plugin a `deluged` reactorban, saját Twisted `Site` a 8113-on; Vue 3 SPA a plugin `data/` alatt.
- Token vagy Deluge user/password auth; 30 napos session token; 1/5/10 szintek.
- Lista a `torrentmanager.torrents`-ből, `get_status(update=False)`, szerveroldali szűrés/szeletelés (833 torrenten p95 8,6 ms).
- SSE az EventManagerre + 1/s diff, 15 s heartbeat; write a core metódusokon és a Label pluginen át.
- PWA, push, fs picker, bulk; a session token memóriában van, restartnál elveszik.

**Átvenni:** szerveroldali szeletelés, virtualizált lista, SSE, PWA, HU/EN, „thin, limited” szemlélet.
**Kerülni:** a daemonba égetett web-servert és a külön portos auth-modellt, ha nem muszáj.

## 3. Célarchitektúra: öt közös alap

1. **Config service** – egy `config.toml` szekciókkal + redb a futásidejű állapotnak; `GetConfig`/`SetConfig` `revision`-nal, atomikus apply, hot reload; hibán a régi snapshot marad. A token redacted. A GUI és a CLI ugyanazt használja. **Kész (2026-11-20):** tipizált séma, env-layering, revision + atomic swap + atomikus fájlírás, RPC és CLI; a label-definíciók/hozzárendelések redb-ben, a watch/scheduler állapot később.
2. **Pause authority** – `pause_reasons: {USER, SCHEDULER, DISK_GUARD, RATIO}`; effektív paused = bármelyik ok aktív. Az engine csak az effektív váltást kapja. Restartnál a `USER` perzisztált, a tranziens reasonök újraszámolódnak. **Kész (2026-11-20):** `TorrentControl` + reason-bitset, perzisztált `paused` kulcs, bulk `set_many`, `Torrent.pause_reasons`, ADR 0007.
3. **Effektív beállítás-feloldás** – torrent override > label policy > session default; a label hozzárendelése/levétele csak újraszámol. A move-on-complete a label policy része; a DiskGuard a célt is látja. **Kész (2026-11-20):** a label-definíciók és -hozzárendelések redb-ben élnek (a CRUD nem írja újra a teljes `config.toml`-t), `Torrent.label`, `ListTorrents.label_filter`, `SortField.LABEL`, auto-match és `Finished` → move; ADR 0008.
4. **Esemény-journal** – perzisztált, sorrendezett domain események (added/finished/removed/error/disk_low/scheduler_changed) + kurzor. Erre épül a hook, a notifications outbox, az Activity fül és a web SSE. Az engine `Finished` eseménye pontosan egyszer jön.
5. **Infohash** – `TorrentStatus.infohash`, perzisztált rekord, proto `Torrent.infohash`; duplicate add → `AlreadyExists{id}`, a daemon a meglévő ID-t adja vissza.

### 3.1 Funkció-térkép

| Funkció | Adatmodell / RPC | Daemon / engine | GUI |
|---|---|---|---|
| Labels | `Label{name,color,policy}`, `Torrent.label`, `SetLabel`, `ListLabels`, CRUD; `ListTorrents.label_filter`, `SortField.LABEL` | effektív feloldás; labelváltásnál limit/move újraszámol; Finished → move | label oszlop, sidebar szűrő, context menü, Settings → Labels (kész, M4-T5) |
| AutoAdd | `WatchDir{path,enabled,save_path,label,add_paused,on_success,move_to,min_free_bytes}`, `[[autoadd]]` a configban; `GetAutoAddStatus`/`ScanAutoAdd` | 1 s reconcile + size/mtime stabilitás; infohash dedupe; perzisztált invalid retry; post-add action; opcionális disk gate | Settings → AutoAdd + „Scan now” + státusz (kész, M5-T1) |
| Execute | `HookSpec{event,command,args,shell,timeout_secs,enabled}` a configban; `GetHookStatus` | esemény-journal (redb, seq+ack), at-least-once, argv+env contract, process-group timeout, max 4 párhuzamos, futásnapló | Settings → Hooks + legutóbbi futások (kész, M5-T2) |
| Notifications | `[notify]` + `[[notify.webhook]]` + `[[notify.telegram]]`; digest/quiet/label filter; `GetNotificationStatus`/`SendTestNotification` | esemény-journal → outbox (redb, dedupe), JSON+HMAC webhook, Telegram Bot API (HTML, 4096, `retry_after`), digest 10/30 s, quiet hours | Settings → Notifications + Test, `ferrent-cli notify test/status` (kész, M5-T4/T5) |
| Scheduler | `[scheduler]` (enabled, timezone, yellow limitek, pause_seeding, week 7×24 `g/y/r`); `GetStatus.scheduler_*` | `chrono-tz` időzóna; abszolút óra-határ (DST-biztos); zöld = session, sárga = limitek, piros = `SCHEDULER` reason; bulk pause/resume 10k < 1 s | Settings → Scheduler rács (kész, M5-T3) |
| DiskGuard | `Disk{mount,fs,total,free,used,zones,torrents,low}`, `[[diskguard.zone]]` (library\|staging), `GetDisks`, zóna CRUD | mounts + statvfs + `st_dev`; **zóna**: library 1:1 címkével + lemezenkénti kötelező staging; add-kor `címke → zóna → st_dev → staging` (azonos lemezen rename, nem copy); címke nélkül globális `download_dir`; guard: hysteresis, csak aktív írók, add/move gate | Settings → Storage: lemez-tábla + zóna-lista, add-dialog letöltési út mező; sidebar „N disks · X free” |
| WebUI | `ferrent-web`: REST/JSON + SSE | axum a daemon gRPC-je előtt; szerveroldali szűrés/szeletelés; 1 Hz coalescing | SPA a webui2-ből: lista, detail, add, label, pause/resume/remove |

### 3.2 Settings modal

900×640, bal oldali fülsáv: Connection, Session, Labels, AutoAdd, Scheduler, Storage, Hooks, Notifications. Minden fül **tábla + egy Save + explicit Reload + dirty jelzés**; nincs soronkénti Apply, nincs auto-apply. A mentés revision-ellenőrzött; hiba a sor mellett.

## 4. Notifications (bővített)

A notifications nem a Deluge Notifications plugin portja: az esemény-journalra épülő, csatornafüggetlen kézbesítés.

- **Forrás:** `TorrentAdded`, `TorrentFinished`, `TorrentError`, `TorrentRemoved`, `DiskLow`/`DiskOk`, `SchedulerChanged`. A hook és a notification ugyanazt a journalt olvassa, külön kurzorral.
- **Szabály:** eseménytípus + label + név regex + quiet hours; csatornánként engedélyezés. Egy esemény több csatornára mehet.
- **Csatornák v1:**
  - **Webhook:** JSON POST, `X-Ferrent-Event`, `X-Ferrent-Delivery`, opcionális `X-Ferrent-Signature: sha256=…` (HMAC a secret-tel); timeout, 5xx/429 retry.
  - **Telegram:** Bot API `sendMessage`; `chat_id`, opcionális `message_thread_id`; HTML/Markdown escape; 4 096 karakter; 429 `retry_after`; token a configban, redacted.
- **Outbox:** redb-ben perzisztált `(event_id, channel_id)` rekord, állapot (`pending/sent/failed/dead`), kísérletek, utolsó hiba; at-least-once, a dedupe kulcs megakadályozza a duplát.
- **Retry:** exponenciális backoff (pl. 5 s, 30 s, 2 min, 10 min, 1 h), max 5; utána `dead` + UI/Activity bejegyzés.
- **Coalescing:** `finished` események digestje (pl. 10 esemény vagy 30 s ablak) a 4 000 torrentes flood ellen; csatornánként konfigurálható.
- **Rate limit:** Telegram ~30 msg/s globális, 1 msg/s chatenként; a worker sorban dolgozik, a webhook párhuzamosítható (max 4).
- **Biztonság:** token/secret nem kerül logba; a webhook URL user-config, SSRF-tudatos (csak http/https, opcionális allowlist); a Telegram token a `0600` configban.
- **Teszt:** Settings → Notifications „Test” gomb és `ferrent-cli notify test`; a unit tesztek fake HTTP szervert használnak, nincs hálózat.
- **Nem cél v1:** email, ntfy, Pushover, per-user címzés, kétirányú Telegram parancsok.

## 5. Rögzített döntések (2026-11-20)

| # | Döntés | Indok |
|---|---|---|
| 1 | Egy `config.toml` szekciókkal + redb runtime state | kevesebb fájl, egy revision, egy validáció |
| 2 | Label: effektív feloldás, nem mutálás; egy torrent egy label (v1) | a kézi override megmarad; a többlabeles modell később |
| 3 | Scheduler piros = mindent pause-ol, `pause_seeding` opcióval | Deluge-parity, a pause authority miatt nem clobberel |
| 4 | Web UI külön `ferrent-web` binary + SPA | a daemon headless marad; külön auth/proxy/release |
| 5 | DiskGuard add gate default `notify`, AutoAdd `defer`; `reject` opció | ne törje el a meglévő automationt; a watch dir megtartja a fájlt |
| 6 | Hook default argv, shell opt-in; 60 s timeout, max 4 párhuzamos | kiszámítható, biztonságosabb, nem blokkol |

## 6. Kockázatok

| Kockázat | Hatás | Kezelés |
|---|---|---|
| Proto-mező minden struct-literal buildert tör | workspace-fordítás elromlik | egy commit minden crate-re; `..Default::default()`; append-only mezőszámok; `cargo check --workspace --all-targets` |
| `move_storage` lassú/blokkoló, cross-FS másolás + recheck | torrent beragad, guard rossz state-et lát | engine szálon, `Moving` state, timeout, azonos FS-en rename; valós adat teszt |
| Pause reason regresszió | torrentek bent ragadnak paused | property teszt: effektív = bármely reason; restart recompute; UI reason |
| Config lost update | GUI/web felülírja egymást | revision + `ABORTED`; egy Save; Reload |
| Scheduler DST/suspend | rossz sáv/limit | `chrono-tz`, next-boundary tick, fake clock teszt |
| Hook/notification security | tetszőleges kód / token szivárgás | argv, nincs shell default, timeout; secret redacted; HMAC; nincs token a logban |
| `statvfs` beragadó NFS-en | guard megáll | `spawn_blocking` + timeout + utolsó payload cache |
| 10k torrent + per-torrent limitek | CPU/Mutex nyomás | csak labelváltáskor applikál, batch engine hívás; mérés |
| Notification flood | Telegram rate limit / spam | digest + coalescing + per-channel rate limit + dead-letter |

## 7. Források

- Deluge 2.2.0: `/usr/lib/python3/dist-packages/deluge/plugins/{AutoAdd-1.8,Label-0.3,Execute-1.3,Scheduler-0.3}.egg`.
- Saját: `~/docker-images/diskguard` (`storage.py`, `guard.py`, `core.py`), `~/docker-images/webui2` (`docs/decisions.md`, `docs/api.md`).
- Ferrent: `ARCHITECTURE.md`, `ROADMAP.md`, `crates/*`.
