# ADR 0001 – libtorrent verzió

- **Státusz:** elfogadva (provisional)
- **Dátum:** 2026-11-19
- **Kontextus:** M0-T3, R1 (4 000+ torrent), ARCHITECTURE §9 / 1. döntés

## Döntés

A `libtorrent-rasterbar` **2.0.x** sorozatát használjuk; a fejlesztői környezet
Debian 13-on **2.0.11**. Az 1.2-est nem építjük forrásból az M0-hoz.

## Indoklás

- A Debian trixie a 2.0.11-et csomagolja (runtime, `-dev`, Python binding); az
  1.2 nem érhető el a disztróban.
- Az 1.2 forrásbuild külön C++ toolchain-t és karbantartást igényel; a
  nyeresége a 4 000 torrentes célhoz nem bizonyított.
- A 2.0 a jelenlegi API (`torrent_status`, `add_torrent_params`, `session`), és
  megvan az út a v2 torrentekhez.
- A `cxx` bridge csak a 2.0 sorozaton stabil felületet használ.

## Mérések (2026-11-19, Debian 13.7, amd64)

| Metrika | Érték | Megjegyzés |
|---|---|---|
| `cargo test -p ferrent-engine --features libtorrent` | 8 teszt, 0.15 s | mock + valódi `.torrent` roundtrip |
| Teszt-bináris csúcs RSS | 17 456 KB (~17 MB) | test harness + egy libtorrent session + egy torrent |
| Valódi `.torrent` add/pause/resume/remove | PASS | `hello.torrent`, 14 KiB payload, hálózat nélkül |
| 4 000 torrent RSS / CPU / indulás | **M0-T4** | a load harness méri |

Az 1.2 vs 2.0 teljes összehasonlítás nem készült el, mert az 1.2 nincs a
disztróban; a döntést a 4 000 torrentes mérés (M0-T4) fogja véglegesíteni.

## Következmények

- Az M0-T4 load harness az ARCHITECTURE §6 büdzsét méri; ha a 2.0.11 nem hozza,
  újranyitjuk ezt az ADR-t (1.2 forrásbuild vagy 2.0 tuning: `file_pool_size`,
  mmap, cache).
- A CI Ubuntu image egy kicsit eltérő 2.0.x-et adhat; a használt API a 2.0
  sorozaton belül stabil.
- Az M0 dinamikusan linkeli a `libtorrent`-et; a statikus linkelés az M3-T2
  feladata.
