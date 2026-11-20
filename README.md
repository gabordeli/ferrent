# Ferrent

Deluge-utód Rustban: `ferrentd` daemon + natív `ferrent-gui` thin client,
`libtorrent` motorral, 4 000 torrentes célra.

- Architektúra: [ARCHITECTURE.md](ARCHITECTURE.md)
- Roadmap: [ROADMAP.md](ROADMAP.md)
- GUI használat: [docs/gui.md](docs/gui.md)
- CLI: [docs/cli.md](docs/cli.md)
- MVP mérés: [docs/measurements/mvp.md](docs/measurements/mvp.md)

## Gyors indítás (Debian 13)

```bash
sudo apt install build-essential pkg-config libtorrent-rasterbar-dev libboost-dev
cargo build --release -p ferrent-daemon --features libtorrent
cargo build --release -p ferrent-cli -p ferrent-gui

FERRENT_DATA_DIR=/var/lib/ferrent \
FERRENT_TOKEN='change-me' \
FERRENT_BIND=127.0.0.1:50051 \
./target/release/ferrentd
```

A daemon első induláskor önaláírt TLS tanúsítványt generál
(`tls-cert.pem`); a GUI első csatlakozáskor ezt fingerprint alapján pineli.
A token a GUI `~/.config/ferrent/gui.toml` fájljában `0600` móddal van.

```bash
./target/release/ferrent-cli --server https://localhost:50051 \
  --ca /var/lib/ferrent/tls-cert.pem --token 'change-me' list
```

## Kipróbálás demo adatokkal

A legegyszerűbb út a mock daemon: nem kell libtorrent, és 4 000 (vagy
10 000) torrenttel indul.

```bash
cargo build --release -p ferrent-gui --example demo_server
./target/release/examples/demo_server --torrents 4000 --port 50129 \
  --data-dir /tmp/ferrent-demo

# másik terminálban
cargo build --release -p ferrent-cli -p ferrent-gui
./target/release/ferrent-gui
```

A GUI profil mezői:

- Server URL: `https://localhost:50129` (a tanúsítvány SAN-je `localhost`,
  ezért `127.0.0.1` helyett ezt használd)
- CA: `/tmp/ferrent-demo/tls-cert.pem`
- API token: `secret`

Első csatlakozáskor a fingerprint dialóguson kell jóváhagyni a daemont.
Utána jön a virtualizált tábla; a szűrő, rendezés, Details panel és a
jobb klikkes műveletek használhatók.

CLI-vel ugyanez:

```bash
./target/release/ferrent-cli --server https://localhost:50129 \
  --ca /tmp/ferrent-demo/tls-cert.pem --token secret --json list --limit 20
```

Valódi, perzisztens daemonhoz:

```bash
FERRENT_DATA_DIR=/tmp/ferrent-data FERRENT_TOKEN=secret \
  FERRENT_BIND=127.0.0.1:50051 ./target/release/ferrentd
```

A GUI-hoz ilyenkor `https://localhost:50051` és
`/tmp/ferrent-data/tls-cert.pem` kell. A `.deb`-et a
`cargo deb -p ferrent-daemon --features libtorrent` állítja elő; a
telepített változat configja `/etc/ferrent/config.toml`, a tokenje
`/etc/ferrent/token`.

## Jelenlegi állapot (MVP, v0.1.0 előkészületben)

- 4 000 torrent betölthető; a state mirror és a gRPC API a 4 000 soros
  terhelés alatt is a keretben marad.
- A GUI virtualizált táblát, szerveroldali szűrést/rendezést, valamint
  pause/resume/remove műveleteket ad.
- Fastresume restart: a daemon leállításkor a resume adatokat a redb-be
  menti; a következő indulás recheck nélkül áll vissza.
- A 4 000 torrentes mérés és a 24 órás soak részletei:
  [docs/measurements/mvp.md](docs/measurements/mvp.md).

## Hibajelentés

A `ferrentd`, `ferrent-cli` és `ferrent-gui` induláskor a Bugsink projekt 5
DSN-jére inicializálja a Sentry klienst; a `FERRENT_BUGSINK_DSN` env var
felülírhatja. A panelek `service` taget kapnak, a kezeletlen hibák és a
Rust panics figyelmeztetés nélkül mennek a Bugsinkbe.

## Ismert korlátok (MVP)

- Nincs `.deb`/APT/Docker csomagolás, Deluge-import és `*arr` API (v1.0).
- A `.torrent` hozzáadás save pathja a **daemon hostján** értelmezett;
  a GUI nem ellenőrzi, hogy létezik-e.
- A GUI `STATE` oszlopa nem rendezhető szerveroldalon; a támogatott
  rendezési kulcsok: id, name, progress, download, upload.
- A token plaintext a `0600` configban (keyring későbbi).
- A TLS tanúsítvány SAN-je `localhost` és `ferrent.local`; távoli gépről
  a `ferrent.local` név (vagy egy megfelelő hosts bejegyzés) szükséges.
- A 24 órás soak a megadott mérési környezetre érvényes, nem párhuzamos
  daemonokra vagy több ezer aktív peerre.

## Licenc

A repó `MIT OR Apache-2.0` döntés előtt áll; a `Cargo.toml` jelenleg a
workspace licencmezőjét használja.
