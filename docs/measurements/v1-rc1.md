# v1.0.0-rc1 – kiadási jegyzőkönyv (M13)

Mérve 2026-11-22. A kiadás a `v1.0.0-rc1` tagen készült (`Cargo.toml`
`1.0.0-rc1`), a tag-pipeline a **6891**.

**Újrakiadás (2026-11-23):** a 6891 a GitHub-feltöltések után a
`release-apt` dispatch 403-ján bukott el, ezért az AppImage és a registry
kimaradt. A javítások (`a4e65ee` nem-fatális dispatch, `d14ff0b` normalizált
asset-név) után a **6917** futott végig: `release-deb` zöld (a 3 `.deb`
asset megvan, 0 új feltöltés), `release-apt dispatched`, registry feltöltve,
majd `release-appimage` zöld. A tag ekkor a `d14ff0b` commitra került (a
kiadási commit + a CI-javítások); a binárisok verziója változatlan.

## Csomagok

| Csomag | Arch | Méret | Hol |
|---|---|---:|---|
| `ferrent_1.0.0~rc1-1_amd64.deb` | amd64 | 27,4 MB | GitHub Release + Generic Registry (`1.0.0-rc1-1`) |
| `ferrent_1.0.0~rc1-1_arm64.deb` | arm64 | – | GitHub Release + Registry (`1.0.0-rc1-1`) |
| `ferrent-gui_1.0.0~rc1-1_amd64.deb` | amd64 | 27,0 MB | GitHub Release + Registry (`1.0.0-rc1-1`) |
| `Ferrent-1.0.0-rc1-x86_64.AppImage` | amd64 | 43 MB | Registry (`ferrent-gui/1.0.0-rc1`) + pipeline artifact |

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

A 2026-11-23-i újrafuttatás (a CLI-vel, ugyanazzal a forrással) **918**
torrentet talált és adott hozzá; a két plusz a közben a Deluge-ba került
torrent. Mind a 918 torrent `paused` maradt, 0 peer.

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

1. **Az import-pause (#81) — javítva** (`1d98821` + `6d9ccc2`): a restore most a
   hozzáadással együtt viszi a `paused` flaget (`auto_managed` nélkül), és a
   daemon a listener előtt egy `refresh_mirror`-t futtat, mert a tábla a
   hozzáadáskori pillanatképet tartotta. Igazolva: **916/916 paused** indítás
   után, 30 s múlva és egy második restart után is; startup 4,0 s. A
   resume-olt torrent RPC-s pause-a utáni stale `show` is javult (#66): a
   pause/resume után a service `refresh_mirror`-t hív. A 2026-11-23-i
   újraindítás után **918/918 paused**, 0 peer.
2. **Deluge plugin-configok**: a `label.conf`/`autoadd.conf`/`execute.conf`/
   `scheduler.conf` két JSON-dokumentum (fejléc + payload), ezért a
   beolvasó `trailing characters` figyelmeztetést ad, és a címkék nem
   jönnek át (0 labelled).
3. **release-apt dispatch — javítva** (`a4e65ee`): a PAT `Actions: write`
   scope-jával a 6917-ben a dispatch 204-gyel lefutott; hiányzó scope esetén
   a job már csak figyelmeztet, nem bukik el.
4. **Asset-feltöltés újrakiadásnál — javítva** (`d14ff0b`): a GitHub a `~`-t
   `.`-ra írja az asset-névben, ezért a „már létezik” ellenőrzés a normalizált
   nevet is nézi, a 422 `already_exists` pedig kihagyás.

## Következő lépés

Az APT repo frissítése az rc1 csomagokkal (a 6917-ből dispatch-elve), utána a
24 órás éles megfigyelés (a kiadási kapu szerint rc1 ≠ v1.0.0).
