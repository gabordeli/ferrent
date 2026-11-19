# ADR 0002 – perzisztencia

- **Státusz:** elfogadva
- **Dátum:** 2026-11-19
- **Kontextus:** M1-T1, R1 (4 000+ torrent), fastresume/címkék/beállítások, crash safety

## Döntés

**redb 2.6.4**, egyetlen adatbázisfájl, `ferrent_state` tábla, sémaverzió `1`.
A store a `RedbStore` implementáció a `StateStore` trait mögött; a `flush`
`Durability::Eventual` módban commitol.

## Alternatívák

- **SQLite (`rusqlite`):** érett, SQL, széles eszköztámogatás; de C függőség
  (bundled vagy rendszer `libsqlite3`), nagyobb API-felület.
- **Flat fájlok / JSON (a Deluge mintája):** egyszerű, de teljes újraírás,
  nincs atomikus commit, 4 000 külön fastresume fájl.

## Mérések (2026-11-19, Debian 13.7, amd64)

| Metrika | redb 2.6.4 (release) | SQLite (Python, WAL/NORMAL) |
|---|---|---|
| Batch 4 000 × 32 B | **32 ms** | 30 ms |
| Open + load 4 000 | **10 ms** | 12 ms |
| `kill -9` nyitott write txn közben | **PASS** | nem mért |
| Függőség | pure Rust | C |

A redb debug buildben a batch ~150 ms (a debug teszt ezért 250 ms-os
budgettel fut); a 100 ms-os cél a release buildre vonatkozik. A SQLite szám
Python `sqlite3`-mal, azonos 4 000 rekorddal, WAL + `synchronous=normal`
mellett készült; a Python overhead miatt indikatív, nem `rusqlite` benchmark.

## Miért redb

- Pure Rust, nincs plusz C toolchain a libtorrenten kívül; egyetlen fájl,
  ACID/MVCC, egyszerű kulcs-érték API a fastresume blobokhoz.
- Release-ben a 100 ms / 2 s budget alatt van, bőven.
- A SIGKILL-teszt bizonyítja, hogy egy nyitott write tranzakció megszakítása
  után az adatbázis olvasható és a korábbi commit megmarad.

## Következmények

- **Egy writer process:** a redb nem több-processzes; a daemon az egyetlen
  tulajdonos.
- **Eventual durability:** process crash után a commit megvan; power loss
  esetén az utolsó másodpercek elveszhetnek. A daemon ilyenkor recheckelhet,
  ami a fastresume-store-nál elfogadható trade-off.
- **Séma-migráció kézi:** a `SCHEMA_VERSION` növelésekor az `open`-ban kell a
  migrációs lépést beiktatni; ismeretlen verzióra `UnsupportedSchema`.
- **Backup:** a daemon leállítása mellett a fájl másolható; később jöhet a
  redb backup API.
- A `crash_writer` bin csak teszt-segéd; a csomagolásból (M3) ki kell zárni.
