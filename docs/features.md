# Funkciók

A Ferrent három futtatható egységből áll: `ferrentd` (daemon, a torrent
motor és az állapot), `ferrent-gui` (natív desktop kliens) és `ferrent-web`
(REST + SSE gateway a böngészős UI-hoz). A CLI (`ferrent-cli`) a daemon
gRPC API-ját vezérli. A részletes Deluge-összevetés:
[docs/feature-matrix.md](feature-matrix.md).

## Daemon (`ferrentd`)

- **Motor:** libtorrent 2.0 (statikusan linkelve a `.deb`-ben), 4 000
  torrentes mérési céllal; a mock `MockEngine` libtorrent nélkül is fut.
- **Állapot:** redb store + fastresume; leállításkor a resume adatok a
  store-ba mennek, induláskor recheck nélkül állnak vissza. A store
  könyvtárazárat tart, ezért egy adatkönyvtáron egy daemon fut.
- **API:** TLS gRPC, önaláírt cert + fingerprint pinning, Bearer token;
  44 RPC (torrentek, fájlok, trackerek, web seedek, címkék, limitek,
  config, import, státusz, watch stream).
- **Konfiguráció:** TOML (`/etc/ferrent/config.toml`), revíziózott
  `GetConfig`/`SetConfig`; a környezeti változók felülírják
  (`FERRENT_*`), a felülírt kulcsok `env_locked`-ként látszanak.
- **Queue:** szerveroldali sorrend, aktív letöltés/seed limitekkel;
  a `queue` szakasz kapcsolható.
- **DiskGuard:** zónák (library) + staging mappák, szabadhely-kapu; a
  letöltés a stagingbe megy, befejezéskor a zónába mozog.
- **Ütemező:** sávos (green/yellow/red) időzóna-kezeléssel, a sávok
  sebesség- és pause-szabályai a configban.
- **RSS-figyelő:** feed poll, cím-regex szűrés, dedup, automatikus add;
  a GUI/CLI előnézetet kérhet (`PreviewRssFeed`).
- **AutoAdd:** figyelt mappák, méret/mtime stabilitás, free-space gate,
  label/move műveletek; a scanner állapota lekérdezhető.
- **Hookok:** esemény (hozzáadva/befejezve/hiba) → parancs, timeout,
  napló (`GetHookStatus`).
- **Értesítés:** webhook/Telegram outbox, újrapróbálkozás, dead-letter.
- **IP-blokklista:** eMule/PeerGuardian és CIDR források, path vagy
  http(s) URL, periodikus frissítés.
- **qBittorrent Web API emuláció:** a `*arr` kliensek által használt
  végpontok ([docs/web-api.md](web-api.md)).
- **Telemetria:** Sentry/Bugsink kliens, a várt hibák (leállt daemon,
  rossz token) nem generálnak issue-t ([docs/security.md](security.md)).

## Desktop GUI (`ferrent-gui`)

- Virtualizált torrent-tábla 4 000 sorral, szerveroldali szűrés,
  rendezés és lapozás; oldalsáv a státusz-számlálókkal és címkékkel.
- Sor-menü: pause/resume/remove, recheck, fájlprioritás, címke, tracker-
  és web seed szerkesztés, áthelyezés, átnevezés.
- Details fülek: Általános, Fájlok, Peer-ek, Trackerek, Statisztika,
  Aktivitás.
- Szerverprofilok fingerprint pinninggel, `gui.toml` (0600).
- Importálás oldal: Deluge forrás előnézettel, szüneteltetett hozzáadás.
- Sötét/világos téma, magyar és angol UI.

## CLI (`ferrent-cli`)

Minden daemon RPC-hez parancs, `--json` minden olvasó parancson, egységes
exit kódok, lokalizált súgó. Teljes referencia példákkal:
[docs/cli.md](cli.md).

## Web (`ferrent-web`)

- REST + SSE gateway a daemon gRPC-ja fölé, token-izolációval; a SPA a
  `webui/` alatt, a gateway szolgálja ki.
- **Web Push:** saját VAPID kulcs + subscription store; torrent kész/hiba
  eseményre böngésző-értesítés (`[push]`, 912a657). A SPA PWA-ként
  telepíthető (manifest + service worker; iOS 16.4+ home-screen PWA).
- Telepítés: [docs/web-install.md](web-install.md); API:
  [docs/web-api.md](web-api.md).

## Import és migráció

- Deluge `state/` + plugin-configok importja (címkék, autoadd, execute,
  scheduler), szüneteltetett hozzáadással, info-hash deduppal:
  [docs/deluge-migration.md](deluge-migration.md). A daemon a `GetStatus`-ban
  jelzi, tesztelt-e a platform (Windows: még nem — a GUI elrejti az oldalt, a
  CLI `import-deluge` továbbra is elérhető).
- uTorrent `resume.dat` + `.torrent` import (`ferrent-cli import-utorrent`),
  `--save-root`/`--path-map` útvonal-képezéssel, szüneteltetve:
  [docs/utorrent-migration.md](utorrent-migration.md).

## Nemzetköziesítés

- A GUI, a CLI és a súgók egy közös katalógusból (`ferrent-i18n`) jönnek;
  alapértelmezés a magyar, `FERRENT_LANG=en` angolra vált. A `--json`
  kulcsok nyelvfüggetlenek.

## Még hiányzik (2026-11-25)

A részletes, kód-mért lista a [feature-mátrix 5. szakaszában](feature-matrix.md#5-a-tényleges-hiányosságok-fontosság-szerint-2026-11-25);
a viselkedési korlátok a [limitations.md](limitations.md)-ben.

**Deluge-paritás:**

- **Streaming** – HTTP stream letöltés közben (a sequential flag már van).
- **Extractor** – automatikus kicsomagolás befejezés után.
- **Torrent-név átnevezése** – a fájlnév-átnevezés kész, a torrent neve nem.
- **Port-teszt / random port + UPnP mapping státusz** – a config kapcsolói
  megvannak, a futásidejű visszajelzés nem.
- **Darabtérkép nézet** – fájlszintű prioritás van, darabszintű nézet nincs.
- **SMTP e-mail értesítés** – webhook/Telegram/Web Push van, e-mail nincs
  (v1-ben nem cél).
- **Prevent-suspend** – desktop-only, alacsony prioritás.
- **Többoldalas kereső** – qBittorrent-szintű kényelem.
- **In-app auto-update** – APT repó és NSIS telepítő van, önfrissítés nincs.

**Kliens-rés:**

- A **web UI-n** nincs Import oldal, nincs torrentenkénti queue-move, és a
  torrent options panel több jelölője (első/utolsó darab, auto-managed,
  ratio, connection) megjelenik, de még nem perzisztál.
- A **GUI** torrent-név átnevezést és darabtérképet nem tud; a queue-move
  viszont megvan (gomb + RPC + CLI). A toolbar tooltipjei (`Add torrent`,
  `Move up in queue`, …) még angolul vannak, a többi chrome kulcsból jön.

**Platform:**

- **Windows:** előnézet (ZIP + NSIS, cross-build); a futásidejű igazolás
  (service, config-út, token-ACL, GUI) hátra van.
- **macOS:** nincs build. **iOS:** parkol, nincs build
  ([platform-support.md](platform-support.md)).
