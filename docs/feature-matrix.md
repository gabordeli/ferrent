# Deluge ↔ Ferrent feature-mátrix

Mérve 2026-11-25 (`main`, d2e3ce2).

**Források:** Deluge `about` (core feature lista) és `plugins` (bundled + 3rd
party) oldala; Ferrent oldalon a `main` (d2e3ce2): a daemon RPC-listája
(`crates/daemon/src/rpc/mod.rs`, 44 RPC), a config séma
(`crates/daemon/src/config/schema.rs`), az engine `settings_pack`
(`crates/engine/src/libtorrent/bridge.cpp`), a CLI parancsfa
(`crates/cli/src/main.rs`), a GUI fülek (`crates/gui/src/main.rs`), a web UI
(`webui/src/`), valamint a `docs/measurements/*` mérések.

**Jelölés:** ✅ built-in · 🟡 libtorrent default, de nincs config/UI felület ·
🔌 Deluge-ban plugin, Ferrentben built-in · ❌ hiányzik a Ferrentből

**Változás a 2026-11-21-es méréshez:** a blocklist, a sequential/super-seeding,
a temp mappa, a tracker/web seed szerkesztés, az IP-filter, a statisztika és a
nyelvesítés sora ✅-ra váltott; új sor a Web Push/PWA. A platformállapot
külön: [docs/platform-support.md](platform-support.md).

## 1. Deluge core feature-ök (`about` oldal)

| Deluge | Ferrent | Megjegyzés |
|---|---|---|
| Core/UI split, daemon mód | ✅ | `ferrentd` + gRPC/TLS, a kliens bármelyik gépről |
| Távoli csatlakozás a daemonhoz | ✅ | TLS + token, tanúsítvány-pinning a GUI-ban |
| Web UI | ✅ | `ferrent-web`: REST + SSE + SPA (PWA-ként telepíthető) |
| Console UI | ✅ | `ferrent-cli` (JSON kimenet, lapozás, minden RPC lefedve) |
| GTK+ UI | ✅ | natív `iced` GUI (nem GTK, de desktop kliens) |
| BitTorrent Protocol Encryption | ✅ | `[network] encryption` = enabled/forced/disabled (M10-T1) |
| Mainline DHT | 🟡 | default be; nincs config/UI kapcsoló (a `settings_pack` nem állítja) |
| Local Peer Discovery (LSD) | 🟡 | default be; nincs config/UI kapcsoló |
| FAST protocol extension | 🟡 | default be; nincs config/UI kapcsoló |
| µTorrent Peer Exchange (PEX) | 🟡 | default be; nincs config/UI kapcsoló |
| UPnP és NAT-PMP | ✅ | `[network] upnp/natpmp` (default: be); a port-mapping állapota nem látszik |
| Proxy support | ✅ | `[network]` HTTP/SOCKS5 (+ jelszavas), peer-proxy is (M10-T1) |
| Web seed | ✅ | metainfóból, libtorrent; a Trackers fülön szerkeszthető, CLI `webseeds` (M12-T2) |
| Private torrents | ✅ | csak tracker, DHT/LSD nélkül (libtorrent) |
| Global és per-torrent sebességlimit | ✅ | globális config + torrentenkénti RPC + label override |
| Configurable bandwidth scheduler | ✅ | `[scheduler]` (idősáv, pause-seeding, limitek) |
| Password protection | ✅ | token + TLS (erősebb, mint a Deluge jelszava) |
| RSS (via Plugin) | ✅ | `[rss]` feed-watcher beépítve, regex szűrő + dedupe (M10-T4) |
| Egyéb libtorrent feature-ök | 🟡 | a libtorrenté, nem exponált |

## 2. Deluge bundled pluginok

| Deluge plugin | Ferrent | Megjegyzés |
|---|---|---|
| AutoAdd – watch folderok | 🔌 | `[autoadd]`, több mappa, label/page |
| Blocklist – IP blocklist import | 🔌 | `[network] blocklist` + beépített eMule/PeerGuardian és CIDR parser, periodikus frissítés, `blocklist` CLI, GUI Network fül (M12-T3) |
| Execute – parancs eseményre | 🔌 | `[[hook]]` (added/complete/removed, timeout, argv) |
| Extractor – kicsomagolás kész után | ❌ | nincs automatikus kicsomagolás |
| Label – címkék + műveletek | 🔌 | label CRUD, auto-match, szín, limit, move-on-complete |
| Notification – email/popup/blink/sound | 🔌 | webhook + Telegram + **Web Push** (böngésző), digest + quiet hours; headless is (912a657); SMTP e-mail nincs (v1-ben nem cél) |
| Scheduler – sávszélesség időhöz | 🔌 | `[scheduler]` |
| Stats – grafikonok | ✅ | Statisztika fül: session-totál + 5 perces sparkline; `stats` CLI (M12-T4) |
| WebUI | 🔌 | beépített `ferrent-web` (külön bináris) |
| Toggle – session paused/resumed | ✅ | bulk pause/resume a kiválasztottakon |

## 3. Deluge 3rd-party pluginok (a lényegesek)

| Plugin | Ferrent | Megjegyzés |
|---|---|---|
| Auto Remove / Plus | 🔌 | ratio + seed-time szabály, `remove`/`stop` akció |
| IfaceWatch | 🔌 | `[network]` + netbind (link-vesztés → pause) |
| Ratio / SeedTime | 🔌 | ugyanaz, per-label felülírással |
| Telegramer | 🔌 | Telegram csatorna beépítve |
| WebAPI (alternatív JSON API) | 🔌 | REST + qBittorrent-kompatibilis API (*arr) |
| Categorise | 🔌 | label auto-match (név/tracker) |
| Copy Completed | 🔌 | label `move_completed_path` / DiskGuard zóna |
| NetWatcher | 🔌 | netbind |
| ltConfig | 🟡 | a config TOML néhány libtorrent-kapcsolót ad (limit, aktív, interfész) |
| Pieces (darab-prioritás) | 🟡 | fájlonkénti prioritás ✅ (M10-T2, RPC + GUI + CLI); darab-térkép nézet nincs; a web options panel „első/utolsó darab” jelölője jelenleg nem perzisztál |
| SequentialDownload | ✅ | sor-menü + `add --sequential` + `set --sequential` + web toggle (M12-T1) |
| PreventSuspend | ❌ | desktop-only, alacsony prioritás |
| Streaming | ❌ | nincs HTTP stream a letöltés közben |
| Default Trackers | ✅ | `[session] default_trackers` (M10-T3); a metainfót nem írja át |
| Randomize / autoport | ❌ | nincs port-randomizálás és nincs port-teszt |
| Updatorr, FlexGet/FlexRSS, YaRSS2 | 🟡 | a beépített `[rss]` watcher kiváltja a YaRSS2-t; FlexGet továbbra is külső |
| uTorrentImport | ✅ | CLI `import-utorrent` + RPC (`resume.dat` + `.torrent`, `save-root`/`path-map`, paused); a GUI Import oldal egyelőre Deluge-only (M14-T8) |
| Smart Move, Browse Button, CopyMagnetURI, IPstatusbar | ❌ | GUI-kényelem |

## 4. Ami a Ferrentben van, a Deluge-ban viszont nem (vagy csak plugin)

Ezek a weboldal „csak a Ferrentben” oszlopához kellenek:

- **DiskGuard zónák**: lemezenkénti szabadhely-kapu + staging → kész-mappa
  mozgatás; a Deluge core-ban nincs lemezterület-ellenőrzés (mérve).
- **Ratio-politika per label**, `remove`/`stop`/seed-time sorrenddel – a
  Deluge-ban plugin.
- **qBittorrent-kompatibilis API** Sonarr/Radarrhoz – a Deluge-nál külön
  plugin/kliens kell.
- **Prometheus `/metrics`** beépítve (opcionális Bearer tokennel).
- **TLS + token auth** a daemon RPC-n (a Deluge jelszavas, titkosítás nélküli
  JSON-RPC-t használ a hálózaton).
- **Deluge- és uTorrent-állapot import** (`import-deluge`: label.conf,
  autoadd, execute, scheduler, .torrent + fastresume; `import-utorrent`:
  resume.dat + .torrent, `save-root`/`path-map`); a `Source` adapter
  kliensfüggetlen, az új kliens egy modul + egy registry-bejegyzés. A
  platform-kapu a daemoné (`import_enabled` a `GetStatus`-ban).
- **Crash-safe state**: redb + checkpoint, `kill -9` után recheck nélkül
  (mért: 227 ms), sérült rekord karanténba.
- **Egy bináris, Python nélkül** (nincs interpreter/plugin-ökoszisztéma).
- **Config revíziók** + GUI workbench (Connection/Session/Labels/Storage/RSS/
  Network), `Save`/`Reload` dirty-kezeléssel.
- **Web Push** a böngészőnek: saját VAPID kulcs, subscription store,
  torrent kész/hiba eseményre (912a657); a SPA PWA-ként telepíthető
  (iOS home-screen; a Web Push iOS 16.4+).
- **Saját tracker** a teszthálóhoz (infra, nem kliens-funkció).
- **Windows port (előnézet)**: daemon + CLI cross-build, ZIP + NSIS; futásban
  még méretlen ([docs/platform-support.md](platform-support.md)).

## 5. A tényleges hiányosságok, fontosság szerint (2026-11-25)

1. **Streaming** – HTTP stream a letöltés közben; a sequential már megvan,
   ez a watch-while-downloading másik fele.
2. **Extractor** – automatikus kicsomagolás befejezés után (Deluge plugin).
3. **Torrent-név átnevezése** – a fájlnév-átnevezés kész (M11-T4), a torrent
   saját neve még nem szerkeszthető.
4. **Port-teszt és random port + UPnP mapping státusz** – a config kapcsolói
   megvannak, a futásidejű visszajelzés nem.
5. **Darab-térkép nézet** – a fájlonkénti prioritás kész, a darab-szintű
   nézet/prioritás nem; a web first/last jelölője még nem perzisztál.
6. **Web UI parity** – nincs Import oldal és torrentenkénti queue-move; a
   torrent options panel több jelölője (first/last, auto-managed, ratio,
   connection) megjelenik, de nem perzisztál.
7. **Prevent-suspend** – desktop-only, alacsony prioritás.
8. **Kereső (több oldal)** – qB-szintű kényelem; plugin-architektúrát igényel.
9. **Beépített auto-update** – APT repó és NSIS telepítő van, in-app updater
   nincs.
10. **SMTP e-mail értesítés** – webhook/Telegram/Web Push van; a Deluge
    Notification plugin e-mailje v1-ben nem cél.

## 6. M10 – Feature parity (lezárva)

| Task | Tartalom | Állapot |
|---|---|---|
| **M10-T1** | Proxy + titkosítás/UPnP config surface | ✅ kész |
| **M10-T2** | Per-file prioritás + szelektív letöltés | ✅ kész |
| **M10-T3** | Torrent készítés + `default_trackers` | ✅ kész |
| **M10-T4** | RSS-watcher | ✅ kész |

Az M11 (téma, i18n, recheck/rename, tracker/web seed) és az M12 (sequential/
super-seeding/temp, IP-filter, stats) is lezárva; a nyitott lista az 5. szakasz.

## 7. Más kliensek — mit érdemes átvenni

Források (2026-11-21, frissítve 2026-11-25): Wikipedia *Comparison of
BitTorrent clients*, `qbittorrent.org` feature lista, a Deluge-oldalak (fent),
valamint a Transmission/Tixati/BiglyBT ismert feature-készlete. A tábla csak a
**kliens-szintű, felhasználói** funkciókat hasonlítja; a protokoll-dolgokat
(DHT, PEX, LSD, µTP, titkosítás) mindegyik tudja.

| Funkció | qBittorrent | Transmission | Deluge | Ferrent | Megjegyzés |
|---|---|---|---|---|---|
| Queue + prioritás | ✅ | ✅ | ✅ | ✅ | daemon-oldali sorrend; GUI + CLI move, a weben torrentenkénti move nincs |
| Sávszélesség-ütemező | ✅ | ✅ | plugin | ✅ | `[scheduler]` |
| RSS + regex szűrő | ✅ | ❌ | plugin | ✅ | M10-T4 |
| Torrent készítés | ✅ | ✅ | ✅ | ✅ | M10-T3 |
| Proxy (HTTP/SOCKS) | ✅ | ❌ | ✅ | ✅ | M10-T1 |
| Web UI + távvezérlés | ✅ | ✅ | plugin | ✅ | `ferrent-web` + gRPC |
| Címkék/kategóriák | ✅ | ❌ | plugin | ✅ | label + auto-match |
| **Force recheck** | ✅ | ✅ | ✅ | ✅ | sor-menü + `ferrent-cli recheck` (M11-T4) |
| **Átnevezés** (torrent/fájl) | ✅ | ✅ | ✅ | 🟡 | fájlnév a torrenten belül (sor-menü + CLI); torrent-név még nincs |
| **Temp / incomplete mappa** | ✅ | ✅ | ✅ | ✅ | `[session] temp_dir`: a payload oda megy, befejezéskor a zónába/download_dir-be mozdul (M12-T1) |
| **Tracker szerkesztés** | ✅ | ✅ | ✅ | ✅ | Trackers fül: hozzáadás/törlés + CLI `trackers --add/--remove` (M12-T2) |
| **Web seed kezelés** | ✅ | rész | rész | ✅ | Trackers fül + `webseeds <id> [--set …]` (M12-T2) |
| **Sequential download** | ✅ | ❌ | ✅ | ✅ | sor-menü + `add --sequential` + web toggle (M12-T1) |
| **Super-seeding** | ✅ | ❌ | ✅ | ✅ | sor-menü, torrentenként + web/CLI (M12-T1) |
| **IP filter** (eMule/P2P) | ✅ | ❌ | plugin | ✅ | `[network] blocklist` + `ip_filter`; CLI `blocklist`, GUI Network fül (M12-T3) |
| **Web Push / PWA értesítés** | ❌ | ❌ | plugin (desktop) | ✅ | `[push]` VAPID + subscription store, kész/hiba esemény (912a657) |
| **Beépített tracker** | ✅ | ❌ | ❌ | 🟡 | külön `ferrent-tracker` bináris |
| **Kereső** (több oldal) | ✅ | ❌ | ❌ | ❌ | qB plugin-os kereső |
| **Statisztika/grafikonok** | ✅ | ✅ | plugin | ✅ | Statisztika fül + `stats` CLI; session-totál (M12-T4) |
| **Téma** (sötét/világos) | ✅ | ✅ | ✅ | ✅ | dark/light + akcentus, futásidőben; GUI és web (M11-T2) |
| **Nyelvesítés** | ✅ ~70 | ✅ | ✅ | ✅ | magyar default + angol; GUI (toolbar tooltipek is), web, CLI chrome, állapotszavak és számlálók (M11-T3, d0c3c38, 0f5ac9d, 55fcbac) |
| Automatikus frissítés | rész | ✅ | ❌ | 🟡 | APT repó (M3-T3) + NSIS; in-app updater nincs |

## 8. Következő kör jelöltjei (2026-11-25)

Haszon/ár szerint rendezve; a „kód” a becsült érintettséget jelzi.

| # | Funkció | Miért | Kód |
|---|---|---|---|
| 1 | **Windows futásidejű igazolás** (service, config-út, token-ACL, GUI) | az „előnézet” csak ezzel lesz támogatott | közepes |
| 2 | **Streaming** (HTTP stream letöltés közben) | watch-while-downloading; a sequential már megvan | közepes–nagy |
| 3 | **Extractor** (auto-unpack befejezés után) | napi haszon scene-release-eknél | közepes |
| 4 | **Web UI parity** (Import oldal, queue-move, options-perzisztálás) | a desktop GUI és a web ne csússzon szét | közepes |
| 5 | **Torrent-név átnevezése** | a fájl-rename kész, ez a maradék | kicsi |
| 6 | **Port-teszt + random port + UPnP mapping státusz** | a config megvan, a visszajelzés hiányzik | kicsi |
| 7 | **Darab-térkép + first/last perzisztálás** | a fájl-prioritás kész, a darab-nézet nem | közepes |
| 8 | **Kereső** (több oldal) | qB-szintű kényelem; plugin-architektúrát igényel | nagy |
| 9 | **In-app auto-update** | APT/NSIS van, önfrissítés nincs | közepes |
| 10 | **SMTP e-mail értesítés** | Deluge-paritás; v1-ben nem cél | kicsi |
