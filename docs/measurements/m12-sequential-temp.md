# M12-T1 – Sequential, super-seeding és temp mappa (mérés)

Dátum: 2026-11-21. Build: `target/release`, saját scratch daemon
(`/tmp/m12-test`, `127.0.0.1:50160`, külön `data_dir`) a temp-méréshez, a live
példány (`/tmp/ferrent-live`, 13 torrent) a flag-ek GUI-s ellenőrzéséhez.

## Mit építettünk

| Réteg | Elem |
|---|---|
| engine | `FfiStatus.sequential` / `super_seeding`, `Session::set_sequential` / `set_super_seeding`, `Command::SetSequential` / `SetSuperSeeding` |
| proto | `Torrent.sequential`, `Torrent.super_seeding`, `AddTorrentRequest.sequential`, `SetSequential`, `SetSuperSeeding` |
| daemon | a két RPC + add utáni `set_sequential`; a mirror frissül, mert a tábla a mirrorból él |
| GUI | a sor-menü 7 elemű: `Sorrendben letöltés` és `Super-seeding` pipával; a pipa a sor állapotából jön |
| CLI/web | `add --sequential`, web body `sequential` |
| config | `[session] temp_dir` (abszolút út), a letöltés oda megy, befejezéskor move |

Commitok: `d82efcc` (engine + proto + RPC + CLI/web), `d0ca0a1` (GUI menü),
`deda756` (`temp_dir`).

## Mérés – flag-ek (live példány, GUI)

| Lépés | Eredmény |
|---|---|
| Sor-menü → `Sorrendben letöltés` (#8) | státuszsor: „Sorrendben letöltés be” |
| Menü újranyitása | `✓ Sorrendben letöltés` (a mirror frissült) |
| Kikapcsolás | „Sorrendben letöltés ki”, a pipa eltűnik |
| `GetTorrent`/protokoll | a `sequential` és `super_seeding` mezők a státuszból jönnek |

## Mérés – temp_dir (scratch daemon)

Config: `download_dir=/tmp/m12-test/downloads`,
`[session] temp_dir=/tmp/m12-test/incomplete`.

| Lépés | Eredmény |
|---|---|
| 2 MiB-os payload a temp mappában, torrent hozzáadása `save_path` nélkül | a torrent a temp alatt jött létre (`incomplete/m12-payload.bin`) |
| Befejezés (`seed_mode = true`, hogy a move önmagában mérhető legyen) | `downloads/m12-payload.bin` létrejött, **`incomplete/` üres** |
| Explicit `save_path` | továbbra is nyer (a temp csak a „nincs út megadva” esetben él) |

**Miért `seed_mode`:** a friss infohash-hez nem volt teljes peer a
swarmban, így a torrent 12,5%-on állt; a `seed_mode` a befejezést
szimulálja, a mért útvonal (temp → végleges hely, temp kiürül) ugyanaz.

## Ami még nincs instrumentálva

- **Sequential darab-sorrend**: a flag bekerül a libtorrenthez és a státuszban
  látszik, de a „darabok sorrendben jönnek / az első fájl progress előbb
  100%” mérés külön, többfájlos swarm-t igényel — a helyi swarm átvitel a
  mérés idején ~100 B/s volt, ezért külön körben érdemes.
- **Super-seeding** a swarm oldalán (a seed nem hirdeti magát teljesnek) még
  nincs megmérve, csak a flag állítása/olvasása.

## Feature-matrix

`docs/feature-matrix.md`: Temp / Sequential / Super-seeding sorok: Ferrent
✅ (temp: `[session] temp_dir` + completion move; sequential/super-seeding:
sor-menü + RPC).
