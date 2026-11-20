# ADR 0007 – Pause authority

- **Státusz:** accepted (2026-11-20)
- **Kontextus:** M4-T3, a scheduler és a DiskGuard előfeltétele
- **Érintett kód:** `crates/daemon/src/control.rs`, `crates/daemon/src/rpc/mod.rs`, `crates/daemon/src/persist.rs`

## Kontextus

A Deluge-ban a Scheduler session-szinten pause-ol, a DiskGuard a saját
halmazát kezeli, a Label plugin pedig torrent-szinten ír. Ezek könnyen
felülírják egymást: egy `resume` visszaindíthatja a kézzel pause-olt torrentet,
vagy egy guard `ok` esemény feloldhatja a scheduler piros sávját.

A Ferrentben három-négy pause-forrás lesz: kézi, scheduler, DiskGuard,
később arány/idő. A forrásoknak nem kell ismerniük egymást, de a végső
állapotnak determinisztikusnak kell lennie, és a kézi pause-nak túl kell
élnie a tranziens forrásokat és a restartot.

## Döntés

1. **Reason bitset torrentenként.** `USER=1`, `SCHEDULER=2`, `DISK_GUARD=4`,
   `RATIO=8`. Az effektív pause = a bitset nem üres.
2. **A motor csak az effektív változást látja.** A `PauseTorrent`/`ResumeTorrent`
   a `USER` bitet állítja; a Scheduler/DiskGuard a sajátját. Ha a bitset
   változatlanul paused marad, nincs engine-hívás.
3. **Perzisztencia:** csak a `USER` bit marad meg restart után
   (`paused` kulcs a redb-ben). A tranziens reasonök induláskor
   újraszámolódnak; a restore után a daemon a `USER` alapján pause-ol.
4. **Bulk:** a `TorrentControl::set_many` visszaadja az effektív állapotot
   váltó id-ket; a daemon egy `pause_many`/`resume_many` hívással viszi át
   őket az engine-be. 10 000 torrenten lineáris, 1 s alatt.
5. **Proto:** `Torrent.pause_reasons` (uint32) adja ki a biteket; a GUI a
   General fülön „Paused by: user, scheduler, …” sorban mutatja.

## Következmények

- A Scheduler és a DiskGuard **soha nem hívja közvetlenül** az engine
  pause/resume-ját; a reason API-t használja.
- A kézi pause megmarad, ha a guard „ok” lesz; a guard „low” a kézi resume
  után újra pause-ol.
- A `PauseTorrent` perzisztencia-hiba esetén az engine már pause-olt lehet;
  a következő restart a perzisztált értéket használja. Ez ritka, de ismert.
- A reason-bitek a felhasználói felé is láthatók; a Deluge-nál ez nem volt
  meg, ott a „miért paused” nem derült ki.

## Alternatívák

- **Session pause:** egyszerű, de nem tudja megkülönböztetni a forrásokat,
  és felülírja a kézi pause-t (ezt használja a Deluge Scheduler).
- **Pause flag a torrenten:** nem komponálható több forrásból.
- **Minden reason külön engine-hívás:** felesleges tranziókat és villogást
  okoz; a bulk API ezt kerüli.
