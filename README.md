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

## Jelenlegi állapot (v0.1.0 / MVP)

- 4 000 torrent betölthető; a state mirror és a gRPC API a 4 000 soros
  terhelés alatt is a keretben marad.
- A GUI virtualizált táblát, szerveroldali szűrést/rendezést, valamint
  pause/resume/remove műveleteket ad.
- Fastresume restart: a daemon leállításkor a resume adatokat a redb-be
  menti; a következő indulás recheck nélkül áll vissza.
- A 4 000 torrentes mérés és a 24 órás soak részletei:
  [docs/measurements/mvp.md](docs/measurements/mvp.md).

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
