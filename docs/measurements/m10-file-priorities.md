# M10-T2 – fájlonkénti prioritás mérés

Mérve 2026-11-21, a live daemonon (`:50129`, `engine=libtorrent`) és a
`ferrent-cli`-vel; a torrentet egy python-libtorrent seeder szolgálta ki
(`/tmp/prio_seed.py`, kétfájlos, 2×2 MiB).

## Végponttól végpontig

| Lépés | Eredmény |
|---|---|
| Add (`ferrent-cli add`) | `id=14`, 2 fájl, mind `priority 4` |
| `files 14 --set 1=skip` | a válasz azonnal `b.bin priority 0` (a daemon a kért értéket tükrözi, mert a libtorrent aszinkron alkalmaz) |
| Letöltés | a seeder **2 100 340 B**-ot adott át, a daemon `100%`, `seeding` |
| Lemez | `/tmp/prio-test/download/prio-test/a.bin` = 2 097 152 B; **b.bin nincs a lemezen** |
| Restart (13 torrent, 159 ms) | `a.bin 4 / b.bin 0` megmaradt, a torrent `seeding 100%` |
| GUI Files fül | a sorok kattinthatók (skipped → normal → high); a kattintás után a panel azonnal a kért értéket mutatja |

Tehát a skip-elt fájl tényleg nem töltődik le, a prioritás fastresume-mal
átmegy a restarton, és a GUI/CLI ugyanazt az RPC-t használja
(`SetFilePriorities`).

## Tesztek

- engine (libtorrent): `file_priorities_round_trip_through_the_bridge` — a
  prioritás eljut a libtorrenthez (0 és 7), ismeretlen id → hiba.
- daemon RPC: `file_priorities_are_validated_against_the_file_count` — a
  hossz-eltérés `InvalidArgument`, üres lista a 0 fájlos mockon `Ok`.
- CLI: `parse_file_priorities` (a nem listázott fájl `normal` marad; rossz
  index/név/tartomány → hiba).

## Ismert korlát

- A `SetFilePriorities` válasza a **kért** értékeket tükrözi (a libtorrent
  aszinkron dolgozza fel); a következő `GetTorrentFiles` a tényleges állapotot
  adja. A GUI ezért kattintás után azonnal frissül.
- A darab-szintű (piece) nézet nem része ennek a tasknak.
