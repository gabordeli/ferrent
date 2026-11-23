# Ferrent

Deluge-utód Rustban: `ferrentd` daemon + natív `ferrent-gui` thin client +
`ferrent-web` gateway, libtorrent motorral, 4 000 torrentes célra. A
daemon és a kliensek TLS gRPC-n beszélnek, a tanúsítványt fingerprint
pineli, az állapot redb-ben él és restart után recheck nélkül áll vissza.

![Ferrent desktop GUI: torrent lista, oldalsáv-számlálók, címkék és
állapotok](docs/images/gui-table.png)

## Fő funkciók

- **Daemon + thin client:** a motor és az állapot egy helyen, a GUI a
  laptopon, a daemon a NAS-on futhat.
- **Virtualizált GUI:** 4 000 soros tábla, szerveroldali szűrés/rendezés/
  lapozás, sor-menü, Details fülek (fájlok, peer-ek, trackerek, statisztika).
- **Teljes CLI:** minden daemon RPC-hez parancs, `--json` kimenet,
  egységes exit kódok, magyar/angol súgó ([docs/cli.md](docs/cli.md)).
- **Web UI:** REST + SSE gateway és böngészős SPA, telefonon is
  ([docs/web-api.md](docs/web-api.md)).
- **DiskGuard:** zónák + staging mappák szabadhely-kapuval; a letöltés a
  stagingbe megy, befejezéskor a zónába mozog.
- **Ütemező, RSS-figyelő, AutoAdd, hookok, webhook/Telegram értesítés.**
- **Címkék** auto-hozzárendeléssel, torrent- és session-limitekkel.
- **IP-blokklista** (eMule/PeerGuardian + CIDR) és qBittorrent-kompatibilis
  API a `*arr` kliensekhez.
- **Deluge-import:** torrentek, címkék és plugin-configok, szüneteltetve
  ([docs/deluge-migration.md](docs/deluge-migration.md)).
- **Fastresume restart** és revíziózott config API; a GUI/CLI nem írja
  felül egymás változását.

Részletes funkcióleírás: [docs/features.md](docs/features.md). A Deluge
feature-mátrix: [docs/feature-matrix.md](docs/feature-matrix.md).

![Ferrent web UI: torrent lista állapotokkal és
címkékkel](docs/images/web-ui.png)

## Összehasonlítás

| | Deluge | qBittorrent | Transmission | Ferrent |
|---|---|---|---|---|
| Felépítés | daemon + thin client | egy folyamat + Web UI | daemon + GUI/web | daemon + natív GUI + web |
| Nyelv | Python | C++/Qt | C | Rust |
| DiskGuard (zóna + staging) | – | – | – | ✔ |
| Beépített Deluge-import | – | – | – | ✔ |
| Licenc | GPL-3.0 | GPL-2.0+ | GPL-2.0/3.0 | MIT OR Apache-2.0 |

A teljes táblázat (RSS, ütemező, API, watch mappa, hookok, platformok)
forrásokkal: [docs/comparison.md](docs/comparison.md).

## Telepítés

| Platform | Útmutató |
|---|---|
| Debian 12/13, Ubuntu 24.04 | [docs/install-debian.md](docs/install-debian.md) (aláírt APT repó vagy `.deb`) |
| Bármely x86_64 Linux (GUI) | [docs/install-appimage.md](docs/install-appimage.md) (AppImage, FUSE nélkül is) |
| Fedora | [docs/install-fedora.md](docs/install-fedora.md) (forrásból) |
| Arch Linux | [docs/install-arch.md](docs/install-arch.md) (forrásból) |
| Docker (fej nélküli daemon) | [docs/install-docker.md](docs/install-docker.md) |
| macOS / Windows | tervezett: [macOS](docs/install-macos.md), [Windows](docs/install-windows.md) |

Demo adatokkal (mock daemon, 4 000 torrent) egy paranccsal kipróbálható:
[docs/demo.md](docs/demo.md). Fejlesztői környezet, tesztek:
[docs/development.md](docs/development.md).

## Jelenlegi állapot (1.0.0-rc1)

Az első kiadásjelölt elkészült: `.deb` (amd64 + arm64) és GUI AppImage a
GitHub Release-en, a szerveren telepítve, a Deluge-ből importált 918
torrenttel, a laptop GUI-ból menedzselve. A mérés és az ismert hibák:
[docs/measurements/v1-rc1.md](docs/measurements/v1-rc1.md), illetve
[docs/limitations.md](docs/limitations.md).

- 4 000 torrent betölthető; a state mirror és a gRPC API a 4 000 soros
  terhelés alatt is a keretben marad
  ([mérés](docs/measurements/mvp.md)).
- A repó egyelőre **privát**; a nyilvános kiadás kapuja hónapokig tartó
  éles torrentes futás, közben a Bugsinkben látszó hibák javításával.

## Dokumentáció

- Architektúra: [ARCHITECTURE.md](ARCHITECTURE.md) · Roadmap:
  [ROADMAP.md](ROADMAP.md)
- GUI: [docs/gui.md](docs/gui.md) · CLI: [docs/cli.md](docs/cli.md)
- Web gateway: [docs/web-api.md](docs/web-api.md) ·
  [docs/web-install.md](docs/web-install.md)
- Deluge-migráció: [docs/deluge-migration.md](docs/deluge-migration.md)
- Biztonság és telemetria: [docs/security.md](docs/security.md)
- CI, tükör és APT repó: [docs/ci.md](docs/ci.md)
- Mérések: [docs/measurements/](docs/measurements/)

## Hibajelentés

A `ferrentd`, `ferrent-cli` és `ferrent-gui` induláskor a Bugsink projekt 5
DSN-jére inicializálja a Sentry klienst; a `FERRENT_BUGSINK_DSN` env var
felülírhatja. A panelek `service` taget kapnak, a kezeletlen hibák és a
Rust panics figyelmeztetés nélkül mennek a Bugsinkbe.

## Licenc

A repó `MIT OR Apache-2.0` kettős licenc alatt áll; a részletek a
[LICENSE](LICENSE) fájlban.
