# Telepítés Dockerrel

Egy image a `ferrentd`-vel és a `ferrent-cli`-vel; GUI és web gateway
nincs benne (a web gateway külön: [docs/web-install.md](web-install.md)).

## Build és indítás

```bash
docker build -f packaging/docker/Dockerfile -t ferrent .
docker run -d --name ferrent \
    -e FERRENT_TOKEN=change-me \
    -p 50129:50129 \
    -v ferrent-data:/var/lib/ferrent \
    ferrent
```

## docker compose

```yaml
services:
  ferrent:
    build:
      context: .
      dockerfile: packaging/docker/Dockerfile
    restart: unless-stopped
    environment:
      FERRENT_TOKEN: change-me
    ports:
      - "50129:50129"
    volumes:
      - ferrent-data:/var/lib/ferrent
      - /mnt/media:/downloads

volumes:
  ferrent-data:
```

## Mit kell tudni

- **Port:** a konténer `0.0.0.0:50129`-en hallgat (a `.deb` loopbacken — a
  gatewayt ugyanarra a hostra várja). Tedd ki a portot, vagy egy Docker
  hálózaton belülről a `ferrent-web`-et állítsd `https://ferrent:50129`-re.
- **Kötet:** a `/var/lib/ferrent` tartalmazza a `state.redb`-t és a TLS
  anyagot. A kötet megőrzi a tanúsítvány fingerprintjét (a GUI ezt pineli)
  és a torrentlistát.
- **Token:** a `FERRENT_TOKEN` kötelező; az entrypoint nélküle el sem indul,
  így a daemon sosem hallgat üres tokennel.
- **Felhasználó:** uid/gid 1000 (`ferrent`); a kötet legyen írható neki.
- **Healthcheck:** 30 másodpercenként `ferrent-cli status` fut TLS-en; két
  hiba után a konténer unhealthy.
- **Letöltések:** alapból `/var/lib/ferrent/downloads`; a médiát kötetként
  tedd be, és a configban állítsd a `download_dir`-t.
- **libtorrent:** a runtime stage a Debian trixie
  `libtorrent-rasterbar2.0` csomagját telepíti, ugyanazt, amihez az image
  épült.

A config a `/etc/ferrent/config.toml` (`packaging/docker/config.toml`):
a DiskGuard és az ütemező ki, a queue be. Saját configot a fájl fölé
mountolva lehet adni — tartsd meg a `bind = "0.0.0.0:50129"` értéket,
különben a publikált port nem ér célt.

## CLI a konténerből

```bash
docker exec ferrent ferrent-cli --server https://localhost:50129 \
  --ca /var/lib/ferrent/tls-cert.pem --token "$FERRENT_TOKEN" status
```

## Frissítés

```bash
docker build -f packaging/docker/Dockerfile -t ferrent .
docker rm -f ferrent && docker run …   # ugyanazzal a kötettel
```

A kötet formátuma verzióközi; a daemon a state-et induláskor olvassa, és a
`libtorrent` fastresume adatot a redb-be menti.
