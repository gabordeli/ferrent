# M14-T8 – uTorrent import (mérés)

Dátum: 2026-11-25. Build: `target/debug` (`ferrentd` libtorrent engine,
`ferrent-cli`), a `main` + a M14-T8 munkafa.

## Környezet

| Elem | Érték |
|---|---|
| uTorrent | 3.6 (build 47390), 32-bit, Wine 10.0 (`wine32:i386`) alatt |
| X | Xvfb `:99` + openbox (a nyelvválasztó és a főablak kezeléséhez) |
| uTorrent app-data | `~/.cache/ferrent-utorrent/drive_c/users/gabor/AppData/Roaming/utorrent` |
| Torrent | `debian-13.8.0-amd64-netinst.iso` (794 820 608 B) |
| Forrás | <https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/> |
| Tracker | `http://bttracker.debian.org:6969/announce` (publikus) |
| Info-hash | `e75a1a6b922815d64cfa9b8904208d3a85143495` |

A uTorrentöt a hivatalos installerrel (`download-new.utorrent.com/.../
utorrent_installer.exe`) telepítettük; a `/S` mód a nyelvválasztó dialógusnál
elakadt, ezért a kicsomagolt `utorrent.exe`-t futtattuk a Wine-prefixben, és
`xdotool`-lal fogadtuk el a nyelvet. A Debian torrentet a CLI-argumentummal
adtuk hozzá; a uTorrent a publikus trackerről el is kezdte tölteni.

## A forrás szerkezete

A `resume.dat` bencoded szótár; a felső szinten `.fileguard`, `rec` és a
torrentenkénti bejegyzések (kulcs: a mellettük lévő `.torrent` fájl neve).
Egy bejegyzésből az import ezeket használja:

| Mező | Jelentés |
|---|---|
| `info` | 20 bájtos info-hash (a `.torrent` SHA-1-jével egyezik) |
| `path` | a torrent gyökerének **teljes** útja (fájl vagy mappa) |
| `caption` | megjelenítendő név (a metasorból is megvan) |
| `labels` | a hozzárendelt címkék listája |
| `trackers` | announce URL-ek (a metasorból is megvan) |

A méréshez tartozó `resume.dat` 10 529 bájt, egy torrenttel; a `.torrent`
fájlok ugyanabban a könyvtárban vannak.

## Importálás

Scratch daemon (libtorrent engine, saját data dir, `127.0.0.1:50170`):

```text
ferrent-cli --server https://localhost:50170 --ca .../tls-cert.pem --token secret \
  import-utorrent --app-dir <uTorrent app-data> --save-root /tmp/utorrent-downloads
```

A `--save-root` a Windows-os `C:\downloads` helyett a daemon hostján létező
útra képez (a `--path-map C:\downloads=/srv/downloads` ugyanezt prefixenként
teszi).

| Lépés | Eredmény |
|---|---|
| első import | `added: 1`, `failed: 0`; a torrent **paused**, `save_path=/tmp/utorrent-downloads` |
| info-hash / név / méret | `e75a1a6b…`, `debian-13.8.0-amd64-netinst.iso`, `794820608` |
| tracker | `http://bttracker.debian.org:6969/announce` (a metasorból) |
| második import | `added: 0`, `already_present: 1` (infohash-dedup) |
| daemon restart | `restored_torrents=1`, az állapot továbbra is `paused` |

## Korlátok

- A uTorrent `have`/`hashed` bittérképét még nem alakítjuk libtorrent resume
  blokká, ezért az első indulásnál a payload ellenőrzése (recheck) várható.
- A `path` Windows-os útvonal; Linux/más hostú daemonnál `save_root` vagy
  `path_map` kell.
- A GUI Import oldala egyelőre a Deluge-ot ismeri; a uTorrent a CLI + RPC
  úton érhető el.
