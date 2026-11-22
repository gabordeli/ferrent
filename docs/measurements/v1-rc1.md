# v1.0.0-rc1 – kiadási jegyzőkönyv (M13)

Mérve 2026-11-22. A kiadás a `v1.0.0-rc1` tagen készült (`Cargo.toml`
`1.0.0-rc1`), a tag-pipeline a **6891**.

## Csomagok

| Csomag | Arch | Méret | Hol |
|---|---|---:|---|
| `ferrent_1.0.0~rc1-1_amd64.deb` | amd64 | 27,4 MB | GitHub Release + Generic Registry (`1.0.0-rc1-1`) |
| `ferrent_1.0.0~rc1-1_arm64.deb` | arm64 | – | GitHub Release |
| `ferrent-gui_1.0.0~rc1-1_amd64.deb` | amd64 | 27,0 MB | GitHub Release |
| `Ferrent-1.0.0-rc1-x86_64.AppImage` | amd64 | 36,7 MB | GitHub Release + Registry (`ferrent-gui/1.0.0-rc1`) |

A `.deb` a Debian-féle `~rc1` alakot használja, a registry útvonala `-rc1`
(a GitLab nem fogadja el a `~`-t a verzióban). **A GitHub Release a `~`-t
`.`-ra cseréli a asset-névben** (`ferrent_1.0.0.rc1-1_amd64.deb`), ezért a
letöltött fájl neve eltér a csomagban lévő verziótól — az `apt install`-t nem
zavarja. A helyi build byte-azonos méretű a CI-ével (27 449 928 B), tehát a
konténeres recept reprodukálható.

**Build idő:** amd64 konténer ~13 perc (statikus libtorrent ~5 perc + cargo
~5 perc + csomagolás), arm64 ~12 perc; a csomagtesztek (33+ assert,
debian:12) a két arch után futnak.

## Szerver (192.168.2.27, Debian 12, x86_64)

- `apt install ./ferrent_1.0.0~rc1-1_amd64.deb`; a `postinst` generálta a
  tokent (`0640 root:ferrent`), a `ferrentd.service` aktív, `engine=libtorrent`.
- Config: `data_dir=/var/lib/ferrent`, `bind=127.0.0.1:50051`,
  `download_dir=/mnt/usbhdd2/_progress` (a Deluge staging útja).
- Restart: `restored_torrents=916`, `startup_ms=998`.
- Deluge: `deluged` a `/var/lib/deluged/config` alatt, `state/` 920 fájl.
  A daemon `ferrent` userként olvas, ezért ACL kellett:
  `setfacl -R -m u:ferrent:rX /var/lib/deluged/config` **és**
  `setfacl -m u:ferrent:r-x /var/lib/deluged` (a szülőn is!).

### Import

`ferrent-cli import-deluge --state-dir /var/lib/deluged/config/state
--config-dir /var/lib/deluged/config`

| Mérőszám | Érték |
|---|---:|
| megtalált torrent | 916 |
| hozzáadva | 916 |
| hiba | 0 |
| futásidő | 3 m 46 s |

## Laptop (a GUI)

- A rc1 `ferrent-gui` bináris a `.deb`-ből kibontva, illetve a
  `Ferrent-1.0.0-rc1-x86_64.AppImage`.
- SSH-alagút: `ssh -N -L 50129:127.0.0.1:50051 root@192.168.2.27`; a profil
  `https://localhost:50129` (a tanúsítvány SAN-ja `localhost`), a CA a
  szerverről másolva, a fingerprint pinnelve.
- `--measure`: **rows=916**, fps 59,9, RSS 128 MB; a státuszsáv
  `csatlakozva · https://localhost:50129 · 916 torrents`.
- Távvezérlés: `resume`/`pause` a CLI-vel az alagúton át működik.

## Ismert hibák (rc1)

1. **Az import-pause nem tapad meg**: a 916 torrent egy része (mért 155)
   `downloading` marad, és a `pause` után is visszaáll; restart után a
   paused állapot nem őrződik meg (a visszaállítás auto-managed-ként indít).
   Reprod: `ferrent-cli pause 1` → `paused 1`, majd `show 1` → `downloading`.
2. **Deluge plugin-configok**: a `label.conf`/`autoadd.conf`/`execute.conf`/
   `scheduler.conf` két JSON-dokumentum (fejléc + payload), ezért a
   beolvasó `trailing characters` figyelmeztetést ad, és a címkék nem
   jönnek át (0 labelled).
3. **release-apt dispatch**: a `GITHUB_RELEASE_TOKEN` a `workflow_dispatch`-nál
   403-at ad (`Resource not accessible by personal access token`) → a PAT-nak
   Actions: write kell. A `.deb`-ek ettől függetlenül felkerültek a release-re.

## Következő lépés

A fenti 3 hiba javítása + az APT repo frissítése az rc1 csomagokkal, utána a
24 órás éles megfigyelés (a kiadási kapu szerint rc1 ≠ v1.0.0).
