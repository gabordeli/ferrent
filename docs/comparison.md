# Kliens-összehasonlítás

A táblázat a 2026-11-23-i állapotot rögzíti; a többi kliens sorai a
hivatalos dokumentációból származnak (lábjegyzet), a Ferrent sorai ebből a
repóból ([docs/features.md](features.md)). A részletes Deluge-mátrix:
[docs/feature-matrix.md](feature-matrix.md).

| | Deluge[1] | qBittorrent[2] | Transmission[3] | rTorrent + ruTorrent[4] | Ferrent |
|---|---|---|---|---|---|
| Nyelv | Python | C++/Qt | C | C++ (+ PHP a ruTorrent) | Rust |
| Felépítés | daemon + thin client[1] | egy folyamat, opcionális fej nélküli `-nox`[2] | daemon + GUI/web[3] | ncurses daemon + web | daemon + natív GUI + web gateway |
| Platform | Linux, macOS, Windows | Linux, macOS, Windows, BSD | Linux, macOS, Windows, BSD | Linux, BSD, macOS | Linux (amd64/arm64); macOS/Windows tervezett |
| Telepítés | disztribúció csomagja | disztribúció csomagja | disztribúció csomagja | disztribúció csomagja | aláírt APT repó; `.deb`; AppImage; Docker[5] |
| Desktop GUI | GTK | Qt | GTK, Qt, macOS | – (ncurses) | `iced` (natív) |
| Web UI | plugin[1] | beépített[2] | beépített[3] | ruTorrent[4] | `ferrent-web` + SPA |
| RPC/API | saját RPC[1] | Web API v2[2] | RPC[3] | XML-RPC/SCGI[4] | TLS gRPC + REST gateway |
| RSS | plugin | beépített | – | ruTorrent plugin | beépített figyelő + előnézet |
| Ütemező | plugin | alternativa sebesség + ütemezés | alt. sebesség ütemezés | ruTorrent plugin | sávos ütemező (green/yellow/red) |
| Lemezterület-kapu | – | szabadhely-ellenőrzés | – | – | DiskGuard zónák + staging |
| Címkék | plugin | kategóriák + címkék | – | ruTorrent kategóriák | címkék, auto-hozzárendelés |
| Watch mappa | plugin (AutoAdd) | beépített | beépített | beépített | AutoAdd stabilitás- és helykapuval |
| Parancs eseményre | Execute plugin | „Run external program” | script | – | hook journal |
| Értesítés | plugin | e-mail/desktop | – | ruTorrent plugin | webhook/Telegram outbox |
| Torrent készítés | GUI | igen | igen | CLI | CLI (`create`) |
| Import más kliensből | – | – | – | – | Deluge import (címkék, pluginok) |
| Licenc | GPL-3.0[1] | GPL-2.0+[2] | GPL-2.0/3.0[3] | GPL-2.0[4] | MIT OR Apache-2.0 |

A „plugin” azt jelenti, hogy a funkció nem a magban, hanem egy hivatalos
vagy népszerű pluginban érhető el; a „–” azt, hogy a hivatalos
dokumentációban nem szerepel.

## Miért daemon + thin client?

A Deluge és a Transmission is így épül fel; a Ferrent ezt viszi tovább:

- a torrent motor és az állapot egy helyen él, a kliensek vékonyak
  (a GUI a laptopon, a daemon a NAS-on futhat);
- a TLS + fingerprint pinning és a Bearer token a daemon oldalán dől el,
  a kliens csak a pinnelt tanúsítványt ismeri;
- ugyanaz az API szolgálja a GUI-t, a CLI-t és a web gatewayt, így a
  funkciók nem csúsznak szét kliensek között.

## Hol több a Ferrent

- DiskGuard: zóna + staging modell szabadhely-kapuval (a Deluge/qBittorrent
  csak a teljes kötet szabad helyét nézi).
- Revíziózott config API (`GetConfig`/`SetConfig`), így a GUI/CLI nem
  írja felül egymás változását.
- Beépített import a Deluge állapotából, a plugin-configokkal együtt.
- A `.deb` statikusan linkeli a libtorrentet, így nincs verziófüggés a
  disztribúciótól.

## Hol kevesebb

- Nincs kicsomagoló, kereső és streaming; a plugin-ökoszisztéma fiatal.
- macOS/Windows build még nincs ([docs/limitations.md](limitations.md)).

## Források

1. Deluge: <https://deluge-torrent.org/> és a plugin-lista
   (<https://deluge-torrent.org/userguide/plugins/>).
2. qBittorrent: <https://www.qbittorrent.org/> és a Web API dokumentáció
   (<https://docs.qbittorrent.org/>).
3. Transmission: <https://transmissionbt.com/> és
   <https://github.com/transmission/transmission>.
4. rTorrent: <https://github.com/rakshasa/rtorrent>; ruTorrent:
   <https://github.com/Novik/ruTorrent>.
5. Ferrent telepítés: [docs/install-debian.md](install-debian.md),
   [docs/install-docker.md](install-docker.md).
