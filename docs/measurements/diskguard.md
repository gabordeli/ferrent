# DiskGuard (M6) – mérések

Mérve 2026-11-20, Debian 13, 12 szál (i7-1355U). A guard a `MockEngine`-en
fut (a payload és a pause-könyvelés a valódi), a fájlrendszer-viselkedés
(`statvfs`, `st_dev`) valódi.

## M1 – Zóna-feldolgozás

`report_with` 1 000 zónát csoportosít `st_dev` szerint, majd lemezenként
egy `statvfs`-t futtat.

| Metrika | Release | Debug | Budget |
|---|---:|---:|---:|
| 1 000 zóna csoportosítása (injektált stat/dev) | **0,5–1,4 ms** | ~4 ms | ≤ 50 ms |
| Valódi `snapshot` (2 zóna, 1 lemez) | < 5 ms | < 10 ms | – |

A valódi scan költsége: **egy `statvfs` lemezenként** + egy `stat` zónánként
(`st_dev`), `spawn_blocking`-ban, 5 s timeouttal; timeout esetén az utolsó
jó payload marad. A `GetDisks` (GUI Storage fül) a zóna nélküli valódi
mountokat is hozzáadja (`include_unzoned`), hogy a felület minden lemezt
fel tudjon ajánlani; a guard csak a zónák lemezeit figyeli.

## M2 – Guard 4 000 torrenten

Egy lemez `low` szintre állítása, majd helyreállás; minden torrent a
`Downloading` állapotban van.

| Metrika | Release | Debug |
|---|---:|---:|
| 4 000 torrent pause-olása (reason + engine) | **24,5–32,4 ms** | 184,1 ms |
| 4 000 torrent resume-olása | **20,4–26,5 ms** | 105,0 ms |

A költség nagy része a torrentenkénti `st_dev` (`device_id_nearest`, 4 000
`stat`) és az engine `pause_many`; release-ben ez 25–32 ms, tehát egy 30 s-es
pollhoz képest elhanyagolható.

Reprodukció:

```bash
cargo test --release -p ferrent-daemon --test diskguard \
  the_guard_handles -- --nocapture
cargo test --release -p ferrent-daemon --lib a_thousand_zones -- --nocapture
```

## M3 – Latencia-korlátok

| Szakasz | Korlát |
|---|---|
| Alacsony lemez észlelése | `poll_secs` + 5 s (a tick 5 s-enként néz órára, a `poll_secs` reload után is él) |
| Pause alkalmazása | a fenti mérés szerint < 35 ms 4 000 torrentnél |
| Move retry tele cél esetén | 300 s, max 5 próba, utána `Error` event |
| Add-kapu (alacsony staging) | azonnali `FAILED_PRECONDITION` (nincs poll) |

## M4 – Éles, több lemezes ellenőrzés (2026-11-20)

A `.27` hoston (root SSH) a három lemez külön ext4 fájlrendszer, a laptop
pedig NFS-en látja őket (`/home/gabor/Storagebox/usbhdd{1,2,3}`, külön
`st_dev`). A payloadot a valódi lemezekre állított zónákkal futtatva:

| Lemez | Eszköz | Méret | Szabad | Zónák |
|---|---|---:|---:|---|
| usbhdd1 | `/dev/sdc1` (st_dev 2081) | 2,7 TiB | 1,4 TiB | `Zene` (library) |
| usbhdd2 | `/dev/sdb1` (st_dev 2065) | 5,4 TiB | 1,9–2,0 TiB | `Filmek`, `Sorozatok` (library), `_progress` (staging) |
| usbhdd3 | `/dev/sdd1` (st_dev 2097) | 5,4 TiB | 1,4 TiB | `xxx_pic`, `xxx-for_seed` (library), `_progress` (staging) |

A `GetDisks` payload a három lemezt külön adta vissza, a zónákat a helyes
lemezhez rendelve; a helyi lemezek (`/`, `/boot`, `/boot/efi` LOW) mellett.

**Éles feloldás:** `ferrent add /tmp/label-test.torrent --label movies` →
`save_path = /home/gabor/Storagebox/usbhdd2/_progress`, azaz a `Filmek`
zóna **azonos lemezén** lévő staging (a `.27`-en `/mnt/usbhdd2/_progress`).
Ez pontosan a kért viselkedés: befejezéskor a move ugyanazon a fájlrendszeren
rename.

**Megfigyelés:** a valós névhasználat eltér a korábbi példáktól
(`xxx_pic`, `xxx-for_seed`, `Zene`; nincs `for-seed`/`xxx-hd` a gyökérben),
és a `usbhdd1`-en **nincs `_progress`** — ha oda címkézett torrent kerül, a
feloldás `MissingStaging`-et ad, amíg a staging zóna létre nem jön. Ez a
modell szándékos viselkedése, nem hiba.

## Korlátok

- A guard `MockEngine`-en fut: a valódi libtorrent pause/resume hívás
  költsége nem benne van (a `TorrentControl` és a döntés igen).
- A 3 FS-es éles ellenőrzés megvan (M4), de NFS-en keresztül; a `.27`-en futó
  daemon saját `statvfs`-e ugyanazokat a számokat adja (a host `df`-je és a
  payload 1–5%-on belül egyezett).
- A mérés egy hoszton, egy felhasználó alatt készült; a `statvfs` hálózati
  FS-en (NFS) lassabb lehet — ezt a timeout fogja le.
