# ADR 0009 – Durable hook event journal

- **Státusz:** accepted (2026-11-20)
- **Kontextus:** M5-T2, Execute hookok
- **Érintett kód:** `crates/daemon/src/events.rs`, `crates/daemon/src/hooks.rs`,
  `crates/daemon/src/rpc/mod.rs`, `crates/daemon/src/config/schema.rs`

## Kontextus

A Deluge Execute plugin fire-and-forget módon futtat parancsokat `added`,
`complete` és `removed` eseményre; nincs timeout, retry, sorrend vagy
perzisztencia. A Ferrentben a hookok akkor is lefussanak, ha a daemon a
futtatás közben áll le, és a futás eredménye (exit, kimenet) visszakereshető
legyen a GUI-ban.

## Döntés

1. **Esemény-journal redb-ben.** Minden hook-releváns tranzíció egy
   `hookevent:<seq>` rekord (JSON), a `hookevent:next` számlálóval. A rekord
   addig él, amíg minden rá illeszkedő hook le nem futott (`ack`).
2. **At-least-once.** Induláskor a függő rekordok újra lefutnak. A hook
   szerződése ezért idempotens: a parancs ugyanazt a torrent-metaadatot kapja
   újra. A sikertelen exit nem számít „nem nyugtázottnak”: a futás
   rögzítése után az esemény ackolódik.
3. **Írási pontok.** `added` az add RPC-ben és az AutoAdd `add_source`-ában
   (a label auto-match után), `removed` a remove RPC-ben a motor eltávolítás
   *előtti* mirror-pillanatképből, `finished`/`error` a `watch` broadcastot
   figyelő taszkban. Így a visszaállított torrentek nem generálnak hamis
   `added` eseményt.
4. **Hook entitás a configban.** `[[hook]]`: `event`, `command`, `args`
   (alapból `{id} {name} {path} {infohash} {label}`), `shell` (opt-in),
   `timeout_secs` (alap 60, max 3600), `enabled`. A config-revízió a szokásos
   `GetConfig`/`SetConfig` úton megy, a GUI Hooks fülén.
5. **Végrehajtás.** argv + `FERRENT_*` környezeti változók; shell csak
   `shell = true` esetén (`/bin/sh -c`, idézett argumentumokkal). Timeoutnál
   a teljes process group kap `SIGKILL`-t (a gyerekek is meghalnak). Egyszerre
   max 4 hook fut; egy esemény hookjai párhuzamosan, az események sorban.
6. **Futásnapló.** `hookrun:<seq>` rekordok (exit, timeout, időtartam,
   stdout/stderr tail 4 KiB), a legutóbbi 200 megőrizve; a GUI
   `GetHookStatus`-szal olvassa (pending darabszám + legutóbbi futások).

## Következmények

- A journal írása a store-mutexen megy keresztül, de az add/remove útvonalon
  rövid; a hook futtatás nem blokkolja a torrent-szálat (tokio task + külön
  process).
- Egy lassú hook legfeljebb a saját eseményét késlelteti; a journal
  növekedése a függő események számától függ.
- A hibás exit nem retry-olódik, csak naplózódik; a felhasználó a GUI-ban
  látja és javíthatja a parancsot.
- A `removed` metaadat a remove pillanatában érvényes; későbbi label-változás
  már nem jelenik meg a hook argv-jában (a label futásidőben oldódik fel).

## Alternatívák

- **Külső spool könyvtár** (fájlok eseményenként): egyszerű, de a GUI-nak
  külön olvasó kellene, és a fájl-nevek ütközhetnek.
- **Per-hook sorok:** külön kurzor hookonként; pontosabb, de több állapot és
  nehezebb „minden hook kész” ack.
