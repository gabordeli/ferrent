# ADR 0008 – Labels in redb

- **Státusz:** accepted (2026-11-20)
- **Kontextus:** M4-T4, label CRUD és policy
- **Érintett kód:** `crates/daemon/src/labels.rs`, `crates/daemon/src/persist.rs`, `crates/daemon/src/rpc/mod.rs`

## Kontextus

A label definíció (szín, move path, limitek, auto-match) és a torrent–label
hozzárendelés is változik futásidőben, a GUI Settings modalból. A config
service egy revisionös, teljes TOML dokumentumot cserél; ha minden label
CRUD a `config.toml`-t írná újra, elvesznének a kommentek, és minden
hozzárendelés a teljes dokumentum revisionjéhez kötődne.

## Döntés

1. **Definíciók redb-ben:** `label:<name>` kulcs, a `Label` TOML-szerű
   szerializációja. A `LabelState` memóriában tartja a térképet.
2. **Hozzárendelés a torrent rekordban:** `torrent:<id>:label`; a
   `PersistedTorrent.label` mezővel restart után is megmarad.
3. **Egy torrent–egy label (v1).** A többlabeles modell később jöhet; a
   `LabelState` most `BTreeMap<TorrentId, String>`.
4. **Effektív limitek:** `TorrentLimits` override > label > 0 (a session
   globális limitje ettől függetlenül érvényes). A label levétele nem
   törli a kézi override-ot.
5. **Egy label törlése** minden hozzárendelést töröl, és a torrentek
   visszaesnek az override/default policy-ra.
6. **Auto-match:** név-regex elsőbbséget élvez a tracker-substringnél; új
   torrentnél az add után, label létrehozásakor/módosításakor a meglévő
   címkézetlen torrentekre is lefut.
7. **Move-on-complete:** a `serve` egy `Finished`-figyelő taszkot indít;
   a label `move_completed_path`-ja alapján `move_storage`-t hív. A
   DiskGuard a label célját a `LabelState`-ből látja.

## Következmények

- A label CRUD nem igényel config-revíziót, és nem írja újra a
  `config.toml`-t.
- A label-definíciók és a hozzárendelések egy helyen (redb) vannak; a
  `GetConfig` nem tartalmazza őket, a `ListLabels`/`Torrent.label` igen.
- A label törlése és a hozzárendelés-törlés több kulcsot ír; a
  `save_torrent_label` külön tranzakció, de a végső állapot idempotens.
- A `SetTorrentLimits` override egyelőre memóriában él; restart után a
  label/default lép életbe. Perzisztálás később.

## Alternatívák

- **Label a `config.toml`-ban:** emberi szerkeszthetőség, de teljes
  dokumentum-rewrite és kommentvesztés minden CRUD-nál.
- **Külön `labels.toml`:** megőrizné a kommenteket, de még egy fájl és
  revision-kezelés; a redb egyszerűbb a futásidejű CRUD-hoz.
