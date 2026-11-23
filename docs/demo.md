# Kipróbálás demo adatokkal

A legegyszerűbb út a mock daemon: nem kell libtorrent, és 4 000 (vagy
10 000) torrenttel indul.

```bash
cargo build --release -p ferrent-gui --example demo_server
./target/release/examples/demo_server --torrents 4000 --port 50129 \
  --data-dir /tmp/ferrent-demo
```

A demo server a `tls-cert.pem`-et a `--data-dir` alá írja; a token fixen
`secret` (a `--token` kapcsoló nincs).

## GUI

```bash
cargo build --release -p ferrent-gui
./target/release/ferrent-gui
```

A profil mezői:

- Server URL: `https://localhost:50129` (a tanúsítvány SAN-je `localhost`,
  ezért `127.0.0.1` helyett ezt használd)
- CA: `/tmp/ferrent-demo/tls-cert.pem`
- API token: `secret`

Ha még nincs mentett profil, induláskor a Servers modal nyílik, ahol a
fenti mezők kitölthetők; mentett profil esetén a GUI azonnal csatlakozik.
Első csatlakozáskor a fingerprint dialóguson kell jóváhagyni a daemont.
Utána jön a virtualizált tábla; a szűrő, rendezés, Details panel és a
jobb klikkes műveletek használhatók.

## CLI

```bash
./target/release/ferrent-cli --server https://localhost:50129 \
  --ca /tmp/ferrent-demo/tls-cert.pem --token secret --json status
./target/release/ferrent-cli --server https://localhost:50129 \
  --ca /tmp/ferrent-demo/tls-cert.pem --token secret --json list --limit 20
```

A `servers add demo --server https://localhost:50129 --ca … --token secret
--use` után elég a `--server demo`.

## Valódi, perzisztens daemon

```bash
cargo build --release -p ferrent-daemon --features libtorrent
FERRENT_DATA_DIR=/tmp/ferrent-data FERRENT_TOKEN=secret \
  FERRENT_BIND=127.0.0.1:50051 ./target/release/ferrentd
```

A GUI-hoz ilyenkor `https://localhost:50051` és
`/tmp/ferrent-data/tls-cert.pem` kell. A `.deb`-et a
`packaging/debian/build.sh` állítja elő (konténerben, statikusan linkelt
libtorrenttel); a telepített változat configja `/etc/ferrent/config.toml`,
a tokenje `/etc/ferrent/token`.
