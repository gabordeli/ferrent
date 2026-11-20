# Automatizálás (M5-T6) – E2E és mérés

Mérve 2026-11-20, Debian 13, 12 szál (i7-1355U). A lánc a `MockEngine`-en
fut, hogy az események és a hozzájuk tartozó időzítés determinisztikus
legyen; a valódi libtorrent motorral a topológia azonos (a
`spawn_engine_watcher` ugyanazokat az `EngineEvent`-eket kapja).

## E2E lefedettség

`crates/daemon/tests/automation_e2e.rs` – négy teszt, mind valós
fájlrendszerrel, valós hook-processzekkel és két helyi fake HTTP szerverrel:

| Teszt | Mit bizonyít |
|---|---|
| `the_automation_chain_runs_from_the_watch_dir_to_the_channels` | watch dir → add → label → limit → move → finished hook → webhook + Telegram |
| `a_restart_keeps_adds_hooks_and_notifications_exactly_once` | journal, outbox és a már hozzáadott torrent túléli a restartot; nincs elveszett vagy duplázott munka |
| `config_reload_swaps_live_rules_and_keeps_the_old_one_on_error` | a `Reload` azonnal cseréli a futó szabályokat; hibás fájl esetén a régi marad |
| `a_hundred_watch_files_land_in_a_thousand_torrent_engine` | 100 fájl 1 000 torrentes motorba (az alábbi mérés) |

A restart teszt két külön tokio runtime-mal fut: az első runtime eldobása
zárja be a redb fájlt és állítja le a daemon taskokat – ez ugyanaz, amit egy
`SIGTERM` + újraindítás jelent.

## M1 – 100 watch fájl 1 000 torrentes motorba

A motor már 1 000 visszaállított torrentet tart; a watch könyvtárba 100
`.torrent` kerül. A mérés a két scan-pass idejét méri (az első csak
fingerprintet rögzít, a második ad hozzá).

| Metrika | Release | Debug | Budget |
|---|---:|---:|---:|
| Első (fingerprint) pass | 0,5–0,7 ms | 1,5–2,5 ms | – |
| Add pass, 100 fájl | **10,1–14,4 ms** | 97,7–129,1 ms | ≤ 2 000 ms |
| Átlagos add | 0,10–0,14 ms | ~1 ms | – |

Reprodukció:

```bash
cargo test --release -p ferrent-daemon --test automation_e2e \
  a_hundred -- --nocapture --test-threads=1
```

A debug érték lassabb, mert minden hozzáadás több `spawn_blocking`
átadást fizet (perzisztencia + label + limit); release-ben ez eltűnik a
zajban.

## M2 – Pickup latencia

A scan 1 s-es reconcile poll, és egy fájlt csak két azonos
méret+mtime fingerprint után ad hozzá (részben másolt fájl kizárása).
Ezért a legrosszabb eset:

| Szakasz | Idő |
|---|---:|
| Fájl megjelenése → első fingerprint | ≤ 1 s |
| Második fingerprint → engine add | ≤ 1 s |
| Feldolgozás (100 fájlnál is) | ≤ 15 ms |
| **Összesen** | **≤ ~2 s** |

A 2 s a Deluge AutoAdd 5 s-os ciklusához képest a port egyik nyeresége; a
mérés felső korlát, a tényleges érték a poll fázisától függ.

## M3 – Restart

| Állítás | Eredmény |
|---|---|
| Journal bejegyzés restart után | megvan (Added + Finished), egyszer fut, ack után nem ismétlődik |
| Outbox bejegyzés restart után | megvan, egyszer küld, utána `pending = 0` |
| Watch fájl újraszkennelése | `duplicate`, nem második add (infohash egyezés a visszaállított motorból) |
| Label hozzárendelés | a perzisztált rekordból visszatöltve (`LabelState::load`) |

## M4 – Move on complete (talált hiba)

Az E2E írta ki, hogy a label `move_completed_path` mozgatás után a mirror
és a perzisztált rekord a **régi** `save_path`-ot őrizte: a GUI rossz helyet
mutatott, restart után pedig a torrent visszakerült volna a régi könyvtárba,
és a watcher „már ott van” ellenőrzése sem működött.

Javítás (`state.rs`, `persist.rs`, `rpc/mod.rs`): sikeres `move_storage`
után a daemon `StateMirror::set_save_path`-tal frissíti a snapshotot és
`persist::save_torrent_path`-tal a redb rekordot. Az E2E ezután a mirror
`save_path`-ját és a restart utáni rekordot is ellenőrzi.

## Korlátok

- A lánc `MockEngine`-en fut; a libtorrent write-viselkedés (tényleges
  fájlmozgatás, `Moving` állapot időtartama) nincs benne.
- A fake HTTP szerver egy kérést szolgál ki; a Telegram `retry_after`
  viselkedését külön unit/integrációs tesztek fedik (`tests/notify.rs`).
- Egy hoszt, egy lemez; a több mountos viselkedés az M6 DiskGuard mérése.
