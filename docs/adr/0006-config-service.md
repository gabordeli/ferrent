# ADR 0006 – Config service

- **Státusz:** accepted (2026-11-20)
- **Kontextus:** M4-T1, a Deluge-plugin port alapja
- **Érintett kód:** `crates/daemon/src/config/`, `crates/daemon/src/rpc/mod.rs`, `crates/cli/src/main.rs`

## Kontextus

A Ferrent eddig négy lapos kulcsot olvasott a `config.toml`-ból
(`data_dir`, `download_dir`, `bind`, `token_file`), hot reload nélkül. A
portolandó funkciók (labels, AutoAdd, scheduler, hooks, notifications,
DiskGuard, web) mind saját konfigurációt igényelnek, és a GUI Settings modal,
valamint a CLI ugyanazt a szerződést kell használja.

Követelmények:

- ismeretlen kulcs ne legyen csendben figyelmen kívül hagyva;
- a GUI/web ne írja felül egymás változását (lost update);
- egy hibás mentés ne hagyja a daemonot félkonfigurált állapotban;
- a változás restart nélkül éljen, de restart után is megmaradjon;
- a titkok ne kerüljenek a `GetConfig` válaszba.

## Döntés

1. **Egy TOML dokumentum, szekciókkal.** A meglévő lapos kulcsok
   megmaradnak; az új funkciók `[session]`, `[queue]`, később `[labels]`,
   `[autoadd]`, `[scheduler]`, `[diskguard]`, `[notify]`, `[hooks]`, `[web]`
   szekciót kapnak.
2. **`deny_unknown_fields`.** A séma minden szinten elutasítja az ismeretlen
   kulcsot; elgépelés = indulási/mentési hiba.
3. **Rétegezés:** fájl < környezet. A `FERRENT_DATA_DIR`,
   `FERRENT_DOWNLOAD_DIR`, `FERRENT_BIND` felülírja a fájlt; a `GetConfig`
   `env_locked` listában jelzi, hogy a fájlbeli érték nem érvényesül.
4. **Revision + atomikus apply.** `GetConfig` visszaadja a revíziót;
   `SetConfig` elavult revízióra `ABORTED`-del válaszol. A sorrend:
   parse → validál → revízió-ellenőrzés → atomikus fájlírás → memória-swap.
   Bármely lépés hibája esetén a régi snapshot marad.
5. **Nyers TOML a válaszban.** A `GetConfig` a fájl szövegét adja vissza
   (kommentek megmaradnak); a kliens a teljes dokumentumot küldi vissza.
6. **Külön config-esemény.** A `Watch` stream `ConfigChanged{revision}`
   üzenetet ad, így a GUI/CLI értesül a változásról.
7. **Titok:** a token továbbra is külön fájlból (`token_file`) vagy env-ből
   jön; a `FileConfig` nem tartalmazza, így a `GetConfig` nem szivárogtatja.

## Következmények

- A `SetConfig` a daemon felhasználójának írhatónak kell lennie a config
  fájlon (a `.deb` csomagolásnál ezt biztosítani kell; lásd M3).
- A `data_dir`, `download_dir` és `bind` változása perzisztálódik, de a futó
  daemon a régi értékeket használja (restart vagy listener-rebind kell); a
  `[session]`/`[queue]` limitek hot-apply-olhatók.
- A `reload` nem írja újra a fájlt, csak felolvassa.
- Az `env_locked` keys esetében a fájlbeli érték módosítható, de az effektív
  érték a restartig a környezeté marad; a kliensnek ezt jelezni kell.
- A `deny_unknown_fields` miatt egy új szekció bevezetése séma-bővítés
  (egy commitban a daemonban és a GUI-ban).

## Alternatívák

- **Külön fájl funkciónként:** több revision, több validációs út, könnyebb
  szekciónként reloadolni; elutasítva az egységes szerződés miatt.
- **Strukturált protobuf config:** típusos, de minden új mezőhöz proto- és
  kliensváltozás kell; a nyers TOML + revision rugalmasabb.
- **Csak redb:** nincs emberi szerkeszthetőség; a TOML marad az igazságforrás.
