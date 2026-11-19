# ADR 0001 – libtorrent verzió

- **Státusz:** elfogadva (engine-szinten; daemon-szintű újraellenőrzés M2-T4)
- **Dátum:** 2026-11-19
- **Kontextus:** M0-T3, M0-T4, R1 (4 000+ torrent), ARCHITECTURE §9 / 1. döntés

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
| `cargo test -p ferrent-engine --features libtorrent` | 8 teszt, 0.13 s | mock + valódi `.torrent` roundtrip |
| Teszt-bináris csúcs RSS | 17 456 KB (~17 MB) | test harness + egy libtorrent session + egy torrent |
| Valódi `.torrent` add/pause/resume/remove | PASS | `hello.torrent`, 14 KiB payload, hálózat nélkül |
| **4 000 torrent (seed)** | **40.4 MB RSS**, **1.73 s betöltés**, **4.70% idle CPU**, **0.25 s leállás** | M0-T4, minden budget PASS |
| 1 Hz-es teljes update | 9.2% idle CPU (5 s settle) | ez indokolta a 2 s-os cadence-et |

Az 1.2 vs 2.0 teljes összehasonlítás nem készült el, mert az 1.2 nincs a
disztróban; a 4 000 torrentes mérés a 2.0.11-et igazolta.

## Következmények

- A progress update **2 s**-onként fut; a state-változások alertből azonnal
  érkeznek. Az aktív torrentek 1 Hz-es frissítése M1/M2 optimalizáció
  (per-torrent update), a 2 s a 4 000 torrentes idle CPU miatt marad.
- A daemon-szintű büdzsét (RPC + state mirror) az M2-T4 méri újra.
- A CI Ubuntu image egy kicsit eltérő 2.0.x-et adhat; a használt API a 2.0
  sorozaton belül stabil.
- Az M0 dinamikusan linkeli a `libtorrent`-et; a statikus linkelés az M3-T2
  feladata.
