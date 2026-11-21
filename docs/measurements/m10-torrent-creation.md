# M10-T3 – torrent készítés + default trackerek mérés

Mérve 2026-11-21, külön teszt-daemonon (`:50160`, `engine=libtorrent`,
`/tmp/m10t3`), hogy a live instance adatait ne keverjük.

## Készítés → hozzáadás

| Lépés | Eredmény |
|---|---|
| Forrás | `album/one.bin` 1 000 000 B + `album/two.bin` 250 000 B (1,25 MB) |
| `ferrent-cli create /tmp/m10t3/album` | **266 bájtos** metainfo, infohash `85c664c0…` (5 × 256 KiB piece) |
| `add album.torrent --save-path /tmp/m10t3/dl` | `id=1`, név `album` |
| `files 1` | 2 fájl: 976,6 KiB + 244,1 KiB (a forrás méretei) |
| `trackers 1` | `udp://tracker.m10t3.test:6969/announce` — a config default trackere, mert a metainfóban nem volt announce URL |

A default trackert a `[session] default_trackers = [...]` adja; a daemon
`add_torrent_params.trackers`-szel fűzi hozzá, a metainfót nem írja át, és a
saját trackerrel rendelkező torrentek listáját nem bántja.

Üres mappa → `Rejected { reason: "the directory has no files" }` (nem panic).

## Két hiba, amit a mérés talált

1. **A piece-határ átlépése fájlok között.** Az első verzió a fájlonkénti
   olvasás végén `filled >= piece_length`-tel zárt, így a két fájl határán a
   hasherbe 463 568 bájt került egy 262 144 bájtos piece-hez: 5 helyett csak
   3 hash készült el. A `create_torrent::generate()` ezt
   `missing or invalid 'hash' entry [libtorrent:175]` kivétellel jelezte, és
   mivel a C++ kivétel kezeletlen volt, a **daemon abortált**. Javítás: a
   ciklus mindig pontosan a hiányzó bájtokat olvassa (`piece_length - filled`),
   így a piece pontosan megtelik.
2. **Kezelés nélküli libtorrent kivétel.** A `make_torrent_from_path` most
   `try/catch`-sel `Rejected` hibát ad vissza; a `generate()` hibája is
   kontextussal jön („generate: … after 3 of 5 pieces”).

## Tesztek

- engine: `create_torrent_from_a_directory_round_trips_through_add` (infohash
  egyezik a hozzáadott torrenttel, 2 fájl, üres mappa hiba),
  `create_torrent_hashes_several_pieces` (5 piece, 1,25 MB, v1 metainfo).
- config: `default_trackers_are_parsed` (TOML → `ResolvedConfig`).
- CLI: `create` (path, `--tracker`, `--piece-length`, `--output`) és
  `trackers <id>` a default trackerek ellenőrzésére.

## GUI

Az Add torrent modal „Create from folder…” gombja: mappaválasztó → a daemon
`CreateTorrent` RPC-je → a metainfo a temp könyvtárba írva → a meglévő
megerősítő modal nyílik meg vele. A gomb megjelenik a modalityban; ebben az
X-sessionben a natív (rfd/portal) választó nem nyílt meg kattintásra, ezért a
kattintásos végpróbát a CLI/RPC útvonal adja.

## Ismert korlátok

- Csak **v1** metainfo készül (a v2 piece-alignelt pad fájlokat a saját
  hashelő nem állítja elő); a v2 külön task lehet.
- A `create` a daemon hostján olvas, tehát távoli kliensnél a mappa a szerveren
  értendő.
