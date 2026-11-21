# Deluge ↔ Ferrent feature-mátrix

Mérve 2026-11-21.

**Források:** Deluge `about` (core feature lista) és `plugins` (bundled + 3rd
party) oldala; Ferrent oldalon a `main` (efa82f4): a daemon RPC-listája
(`crates/daemon/src/rpc/mod.rs`), a config séma
(`crates/daemon/src/config/schema.rs`), az engine `settings_pack`
(`crates/engine/src/libtorrent/bridge.cpp`), valamint a
`docs/measurements/*` mérések.

**Jelölés:** ✅ built-in · 🟡 libtorrent default, de nincs config/UI felület ·
🔌 Deluge-ban plugin, Ferrentben built-in · ❌ hiányzik a Ferrentből

## 1. Deluge core feature-ök (`about` oldal)

| Deluge | Ferrent | Megjegyzés |
|---|---|---|
| Core/UI split, daemon mód | ✅ | `ferrentd` + gRPC/TLS, a kliens bármelyik gépről |
| Távoli csatlakozás a daemonhoz | ✅ | TLS + token, tanúsítvány-pinning a GUI-ban |
| Web UI | ✅ | `ferrent-web`: REST + SSE + SPA |
| Console UI | ✅ | `ferrent-cli` (JSON kimenet, lapozás) |
| GTK+ UI | ✅ | natív `iced` GUI (nem GTK, de desktop kliens) |
| BitTorrent Protocol Encryption | ✅ | `[network] encryption` = enabled/forced/disabled (M10-T1) |
| Mainline DHT | 🟡 | default be |
| Local Peer Discovery (LSD) | 🟡 | default be |
| FAST protocol extension | 🟡 | default be |
| µTorrent Peer Exchange (PEX) | 🟡 | default be |
| UPnP és NAT-PMP | ✅ | `[network] upnp/natpmp` (default: be), a státusz nem látszik |
| Proxy support | ✅ | `[network]` HTTP/SOCKS5 (+ jelszavas), peer-proxy is (M10-T1) |
| Web seed | ✅ | metainfóból, libtorrent |
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
| Blocklist – IP blocklist import | ❌ | nincs `ip_filter` betöltés |
| Execute – parancs eseményre | 🔌 | `[[hook]]` (added/complete/removed, timeout, argv) |
| Extractor – kicsomagolás kész után | ❌ | nincs |
| Label – címkék + műveletek | 🔌 | label CRUD, auto-match, szín, limit, move-on-complete |
| Notification – email/popup/blink/sound | 🔌 | webhook + Telegram, digest + quiet hours (headless is!) |
| Scheduler – sávszélesség időhöz | 🔌 | `[scheduler]` |
| Stats – grafikonok | ❌ | nincs statisztika-nézet |
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
| Pieces (darab-prioritás) | 🟡 | fájlonkénti prioritás ✅ (M10-T2, RPC + GUI + CLI); darab-nézet még nincs |
| SequentialDownload | ❌ | nincs sequential flag |
| PreventSuspend | ❌ | desktop-only, alacsony prioritás |
| Streaming | ❌ | nincs HTTP stream a letöltés közben |
| Default Trackers | ✅ | `[session] default_trackers` (M10-T3); a metainfót nem írja át |
| Randomize / autoport | ❌ | nincs port-randomizálás/teszt |
| Updatorr, FlexGet/FlexRSS, YaRSS2 | 🟡 | a beépített `[rss]` watcher kiváltja a YaRSS2-t; FlexGet továbbra is külső |
| uTorrentImport | ❌ | csak Deluge-import van (`M8-T1`) |
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
- **Deluge-állapot import** (`import-deluge`: label.conf, autoadd, execute,
  scheduler, .torrent + fastresume).
- **Crash-safe state**: redb + checkpoint, `kill -9` után recheck nélkül
  (mért: 227 ms), sérült rekord karanténba.
- **Egy bináris, Python nélkül** (nincs interpreter/plugin-ökoszisztéma).
- **Config revíziók** + GUI workbench (Connection/Session/Labels/Storage),
  `Save`/`Reload` dirty-kezeléssel.
- **Saját tracker** a teszthálóhoz (infra, nem kliens-funkció).

## 5. A tényleges hiányosságok, fontosság szerint

1. ~~**Per-file prioritás + szelektív letöltés**~~ – **kész (M10-T2)**: RPC +
   GUI Files (kattintható sorok) + CLI; a skip-elt fájl nem töltődik le, a
   prioritás restart után is megmarad (`docs/measurements/m10-file-priorities.md`).
2. ~~**Proxy támogatás** (HTTP/SOCKS5 + peer-proxy)~~ – **kész (M10-T1)**:
   `[network]` proxy + encryption + UPnP/NAT-PMP, SOCKS5-tel mérve
   (`docs/measurements/m10-network-settings.md`).
3. ~~**RSS-watcher**~~ – **kész (M10-T4)**: `[rss]` feedek, regex szűrő,
   tartós dedupe, `ferrent-cli rss` + Settings → RSS fül
   (`docs/measurements/m10-rss-watcher.md`).
4. **IP blocklist** – privát trackereknél/anti-leechhez; közepes.
5. ~~**Torrent készítés + default trackerek**~~ – **kész (M10-T3)**:
   `CreateTorrent` RPC + `ferrent-cli create` + GUI gomb; a default trackert a
   `[session] default_trackers` adja (`docs/measurements/m10-torrent-creation.md`).
6. **Sequential download** – kicsi, streameléshez/watch-while-downloading.
7. **Statisztika-nézet / session összesítés** – GUI-kényelem.
8. **Pieces nézet, streaming, prevent-suspend** – alacsony prioritás.

## 6. Javaslat: M10 – Feature parity

| Task | Tartalom | Miért |
|---|---|---|
| **M10-T1** | Proxy + titkosítás/UPnP config surface (engine settings → TOML → GUI Advanced) | privacy, a Deluge core-ban van |
| **M10-T2** | Per-file prioritás + szelektív letöltés (RPC + GUI Files + CLI) | a legnagyobb funkcionális lyuk |
| **M10-T3** | Torrent készítés + `default_trackers` | kicsi, önálló, azonnal használható |
| **M10-T4** | RSS-watcher (feed → szűrő → add, label, dedupe) | beépített, Deluge-ban plugin |

Az IP blocklist, a sequential letöltés és a stats/pieces nézet egy következő
kör (M11) jelöltje; a streaming és a prevent-suspend továbbra is opcionális.

## 7. Más kliensek — mit érdemes átvenni

Források (2026-11-21): Wikipedia *Comparison of BitTorrent clients*,
`qbittorrent.org` feature lista, a Deluge-oldalak (fent), valamint a
Transmission/Tixati/BiglyBT ismert feature-készlete. A tábla csak a
**kliens-szintű, felhasználói** funkciókat hasonlítja; a protokoll-dolgokat
(DHT, PEX, LSD, µTP, titkosítás) mindegyik tudja.

| Funkció | qBittorrent | Transmission | Deluge | Ferrent | Megjegyzés |
|---|---|---|---|---|---|
| Queue + prioritás | ✅ | ✅ | ✅ | ✅ | daemon-oldali sorrend |
| Sávszélesség-ütemező | ✅ | ✅ | plugin | ✅ | `[scheduler]` |
| RSS + regex szűrő | ✅ | ❌ | plugin | ✅ | M10-T4 |
| Torrent készítés | ✅ | ✅ | ✅ | ✅ | M10-T3 |
| Proxy (HTTP/SOCKS) | ✅ | ❌ | ✅ | ✅ | M10-T1 |
| Web UI + távvezérlés | ✅ | ✅ | plugin | ✅ | `ferrent-web` + gRPC |
| Címkék/kategóriák | ✅ | ❌ | plugin | ✅ | label + auto-match |
| **Force recheck** | ✅ | ✅ | ✅ | ❌ | egy torrent újraellenőrzése |
| **Átnevezés** (torrent/fájl) | ✅ | ✅ | ✅ | ❌ | torrent-név és fájlnév |
| **Temp / incomplete mappa** | ✅ | ✅ | ✅ | 🟡 | DiskGuard staging zóna + move-on-complete; nincs globális „incomplete" path |
| **Tracker szerkesztés** | ✅ | ✅ | ✅ | ❌ | tracker hozzáadás/törlés torrentenként |
| **Web seed kezelés** | ✅ | rész | rész | 🟡 | libtorrent tudja, nincs felület |
| **Sequential download** | ✅ | ❌ | ✅ | ❌ | „download in order" |
| **Super-seeding** | ✅ | ❌ | ✅ | ❌ | seed-mód torrentenként |
| **IP filter** (eMule/P2P) | ✅ | ❌ | plugin | ❌ | blokklista + `ip_filter` |
| **Beépített tracker** | ✅ | ❌ | ❌ | 🟡 | külön `ferrent-tracker` bináris |
| **Kereső** (több oldal) | ✅ | ❌ | ❌ | ❌ | qB plugin-os kereső |
| **Statisztika/grafikonok** | ✅ | ✅ | plugin | ❌ | session + torrent szint |
| **Téma** (sötét/világos) | ✅ | ✅ | ✅ | ✅ | dark/light + akcentus, futásidőben (M11-T2) |
| **Nyelvesítés** | ✅ ~70 | ✅ | ✅ | ❌ | ma csak magyar/angol kevert |
| Automatikus frissítés | rész | ✅ | ❌ | 🟡 | APT repó (M3-T3) |

## 8. Következő kör jelöltjei (M11/M12)

Haszon/ár szerint rendezve; a „kód” oszlop a becsült érintettséget jelzi.

| # | Funkció | Miért | Kód | Javaslat |
|---|---|---|---|---|
| 1 | **Bugsink-hibák javítása** (FERRENT-4 redb lock, FERRENT-5 transport, FERRENT-11 `unwrap`) | a kiadási kapu része; valódi felhasználói hibák | kicsi–közepes | M11-T1 (hardening) |
| 2 | ~~**Téma: sötét/világos + akcentus**~~ – **kész (M11-T2)**: `Palette` + accessorok, Settings → Appearance, `gui.toml` (`docs/measurements/m11-theme.md`) | — | — | kész |
| 3 | **Nyelvesítés (hu default, en)** | a user kérése; induláshoz elég a GUI+CLI szövegei | közepes | M11-T3 |
| 4 | **Force recheck + átnevezés** | alap kliens-paritás; a set location **már van** (`MoveTorrent` → `move_storage`) | közepes | M11-T4 |
| 5 | **Tracker szerkesztés + web seed** | privát trackereknél napi haszon | közepes | M11-T5 |
| 6 | **Sequential + super-seeding + temp mappa** | watch-while-downloading; temp mappa a DiskGuardhoz illik | közepes | M12-T1 |
| 7 | **IP filter (eMule/P2P formátum)** | anti-leech / privát tracker | kicsi–közepes | M12-T2 |
| 8 | **Statisztika-nézet** | session-összesítés, grafikon | közepes | M12-T3 |
| 9 | **Kereső (több oldal)** | qB-szintű kényelem; plugin-architektúrát igényel | nagy | M13 jelölt |

A #1 a legfontosabb: a #22 lezárása után a nyitott Bugsink-tételek a kiadási
kapu (hónapokig tartó éles futás) egyetlen ismert akadálya.
