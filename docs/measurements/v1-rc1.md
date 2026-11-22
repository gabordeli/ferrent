# v1.0.0-rc1 – kiadási jegyzőkönyv (M13)

Mérve 2026-11-22. A kiadás a `v1.0.0-rc1` tagen készült (pipeline 6887), a
`Cargo.toml` verziója `1.0.0-rc1`.

## Csomagok

| Csomag | Arch | Hol |
|---|---|---|
| `ferrent_1.0.0~rc1-1_amd64.deb` | amd64 | Generic Registry + GitHub Release |
| `ferrent_1.0.0~rc1-1_arm64.deb` | arm64 | Generic Registry + GitHub Release |
| `ferrent-gui_1.0.0~rc1-1_amd64.deb` | amd64 | Generic Registry + GitHub Release |
| `Ferrent-1.0.0-rc1-x86_64.AppImage` | amd64 | Generic Registry (`ferrent-gui`) |

A `.deb` verzió a Debian-féle `~rc1` alakot használja (a `1.0.0` elé rendeződik),
az AppImage neve a Cargo verziót követi.

## Szerver (192.168.2.27, Debian 12, x86_64)

- Telepítés a `ferrent_1.0.0~rc1-1_amd64.deb`-ből (`apt install ./…`), a
  `postinst` generálja a tokent, a `ferrentd.service` aktív.
- Deluge: `deluged` a `/var/lib/deluged/config` alatt, állapot
  `/var/lib/deluged/config/state` (916 `.torrent`), letöltési út
  `/mnt/usbhdd2/_progress`.
- Import: `ferrent-cli import-deluge --state-dir … --dry-run`, majd élesben;
  a torrentek **paused** állapotban kerülnek be.

## Laptop

- `ferrent-gui` a szerverhez SSH alagúton (`localhost:50129` → a szerver
  daemonja), a tanúsítvány SAN-ja `localhost`, ezért nem IP-re kötünk.
- A profil a `~/.config/ferrent/gui.toml`-ban: server/CA/token/fingerprint.

## Mérések

A pontos számok (build idő, csomagméretek, import darabszám, GUI fps/RSS) a
kiadás után kerülnek ide.
