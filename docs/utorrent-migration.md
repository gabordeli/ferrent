# uTorrent → Ferrent migráció

A uTorrent `resume.dat`-jéből és a mellette lévő `.torrent` fájlokból a
torrentek átemelhetők. Ugyanaz a `Source` adapter szolgálja ki a CLI-t és a
daemon `ImportPreview`/`ImportState` RPC-jét, így az előnézet és a
végrehajtás nem térhet el. A Deluge-útvonal:
[deluge-migration.md](deluge-migration.md).

## Hol van a forrás

A uTorrent Classic app-data könyvtára (a `resume.dat` mellett vannak a
`.torrent` fájlok):

| Környezet | Példa |
|---|---|
| Windows | `%APPDATA%\uTorrent` |
| Wine | `<prefix>/drive_c/users/<user>/AppData/Roaming/utorrent` |
| Linux uTorrent Server | a kliens saját config könyvtára (a `resume.dat` ott van) |

A forrás a **daemon hostján** értelmezett útvonal; a GUI a saját gépén nem
látja. A megállított (vagy legalább nem író) kliens állapota a biztonságos.

## Mit visz át

- **Torrentek**: minden `resume.dat`-bejegyzés, amelyhez tartozó `.torrent`
  olvasható; az info-hash, név, méret és trackerek a metasorból jönnek.
- **Mentési út**: a `resume.dat` `path` mezője a torrent gyökere (fájl vagy
  mappa); az import a szülő könyvtárat adja a daemonnak.
- **Címkék**: a bejegyzés `labels` listája; a hiányzó címkék létrejönnek.
- **Dedup**: a daemonban már létező info-hash `skipped`, nem hiba.
- **Állapot**: a torrentek **paused** kerülnek be.

## Amit nem visz át

- **uTorrent resume bittérkép**: a `have`/`hashed` mezőket még nem alakítjuk
  libtorrent resume blokká, ezért az első indulásnál recheck várható.
- **Ratio, feltöltés/letöltés, queue-sorrend**: a uTorrent számlálói nem
  jönnek át.
- **Beállítások** (sebességlimitek, port, proxy): a Ferrent `config.toml`-ja
  marad az igazság.

## Útvonal-képezés

A uTorrent Windows-os `path` értéke a Linux daemonon nem létezik. Két opció:

- `--save-root <dir>`: **minden** torrent mentési útját erre cseréli;
- `--path-map FROM=TO`: prefixenként képez (többször is megadható), pl.
  `--path-map 'C:\downloads=/srv/torrents'`.

## Lépések

1. Állítsd le a uTorrentet (a `resume.dat` ne íródjon közben).
2. Előnézet daemon nélkül:
   `ferrent-cli import-utorrent --app-dir <app-data> --save-root <dir> --dry-run`
3. Ellenőrizd a listát (név, info-hash, méret, címke, mentési út), majd
   importáld: ugyanaz a parancs `--dry-run` nélkül (a daemon `--server`,
   `--ca`, `--token` kell).
4. A torrentek `paused` állapotban jelennek meg; a szükségeseket indítsd,
   vagy állítsd be a címkéket/limiteket előbb.
5. Csak **publikus trackeres** torrenttel próbáld élesben: resume után az add
   announce-ol, és egy privát tracker kitilthat.

## Mérés

A valódi uTorrent 3.6 + Debian ISO kör:
[m14-utorrent-import.md](measurements/m14-utorrent-import.md).
