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

## M4 – Éles, több lemezes ellenőrzés (nyitott)

A `.27` hoston a `/mnt/usbhdd1..3` három külön fájlrendszer, de a jelen
sessionből SSH nem elérhető (`gabor@192.168.2.27` → publickey denied), ezért
a három lemezes éles ellenőrzés még nyitott. A lépések, ha a hozzáférés megvan:

```bash
# 1. a valós mount-tábla és a zónák
findmnt -l -o TARGET,SOURCE,FSTYPE,SIZE,AVAIL /mnt/usbhdd1 /mnt/usbhdd2 /mnt/usbhdd3
ls -1d /mnt/usbhdd*/*/

# 2. a payload
ferrent disks --json   # vagy: grpcurl a GetDisks-re

# 3. low szimuláció: egy lemezre dd-vel tölteni, majd figyelni a
#    DiskLow eventet és a csak-írók pause-olását
```

A zóna-feloldás helyességét a T1/T2 tesztjei fedik (valós `st_dev`,
cross-device `MissingStaging`), a guard viselkedését a T3 tesztje két
fájlrendszerrel (build FS + tmpfs).

## Korlátok

- A guard `MockEngine`-en fut: a valódi libtorrent pause/resume hívás
  költsége nem benne van (a `TorrentControl` és a döntés igen).
- A 3 FS-es, valódi lemezes mérés a `.27` hoston még hátra van (M4).
- A mérés egy hoszton, egy felhasználó alatt készült; a `statvfs` hálózati
  FS-en (NFS) lassabb lehet — ezt a timeout fogja le.
