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
| BitTorrent Protocol Encryption | 🟡 | libtorrent default (prefer), nincs ki/be kapcsoló |
| Mainline DHT | 🟡 | default be |
| Local Peer Discovery (LSD) | 🟡 | default be |
| FAST protocol extension | 🟡 | default be |
| µTorrent Peer Exchange (PEX) | 🟡 | default be |
| UPnP és NAT-PMP | 🟡 | default be, a státusz nem látszik |
| Proxy support | ❌ | se HTTP, se SOCKS5, se peer-proxy |
| Web seed | ✅ | metainfóból, libtorrent |
| Private torrents | ✅ | csak tracker, DHT/LSD nélkül (libtorrent) |
| Global és per-torrent sebességlimit | ✅ | globális config + torrentenkénti RPC + label override |
| Configurable bandwidth scheduler | ✅ | `[scheduler]` (idősáv, pause-seeding, limitek) |
| Password protection | ✅ | token + TLS (erősebb, mint a Deluge jelszava) |
| RSS (via Plugin) | ❌ | nincs feed-watcher |
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
| Pieces (darab-prioritás) | ❌ | nincs darab-nézet |
| SequentialDownload | ❌ | nincs sequential flag |
| PreventSuspend | ❌ | desktop-only, alacsony prioritás |
| Streaming | ❌ | nincs HTTP stream a letöltés közben |
| Default Trackers | ❌ | nincs „minden új torrenthez” tracker-lista |
| Randomize / autoport | ❌ | nincs port-randomizálás/teszt |
| Updatorr, FlexGet/FlexRSS, YaRSS2 | ❌ | feed-alapú automatizálás; a *arr a qBittorrent API-n át megy |
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

1. **Per-file prioritás + szelektív letöltés** – a Deluge GTK alapja; nálunk a
   Files fül csak olvas. Nagy haszon, kicsi költség (az engine már ismeri a
   prioritást).
2. **Proxy támogatás** (HTTP/SOCKS5 + peer-proxy) – privacy/VPN felhasználóknál
   alapelvárás, a config és az engine-kapcsolat megvan hozzá.
3. **RSS-watcher** – a Deluge-ban is csak plugin (YaRSS2, a TRaSH szerint
   halott); beépítve ez valódi differenciáló.
4. **IP blocklist** – privát trackereknél/anti-leechhez; közepes.
5. **Torrent készítés + default trackerek** – kicsi, hasznos (a bridge
   `make_torrent` már létezik).
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
