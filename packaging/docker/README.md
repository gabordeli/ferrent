# Docker image (headless daemon)

A single image with `ferrentd` and `ferrent-cli`; no GUI, no web gateway.

```bash
docker build -f packaging/docker/Dockerfile -t ferrent .
docker run -d --name ferrent \
    -e FERRENT_TOKEN=change-me \
    -p 50129:50129 \
    -v ferrent-data:/var/lib/ferrent \
    ferrent
```

- **Port:** the container binds `0.0.0.0:50129` (the packaged `.deb` binds
  loopback — the gateway is expected on the same host). Publish it, or keep it
  on a Docker network and point `ferrent-web` at `https://ferrent:50129`.
- **Volume:** `/var/lib/ferrent` holds `state.redb` and the TLS material.
  Keeping the volume keeps the certificate fingerprint (the GUI pins it) and the
  torrent list.
- **Token:** `FERRENT_TOKEN` is required; the entrypoint refuses to start
  without it, so the daemon never listens with an empty secret.
- **User:** uid/gid 1000 (`ferrent`); make sure the volume is writable by it.
- **Healthcheck:** every 30 s the image runs `ferrent-cli status` over TLS
  against the daemon; two failures mark the container unhealthy.
- **Downloads:** `/var/lib/ferrent/downloads` unless the config says otherwise;
  mount the media path you actually want.
- **libtorrent:** the runtime stage installs `libtorrent-rasterbar2.0` from
  Debian trixie, the same version the image is built against.

The config is `/etc/ferrent/config.toml` (`packaging/docker/config.toml`):
diskguard and the scheduler are off, the queue is on. To change settings,
mount your own file over it and keep `bind = "0.0.0.0:50129"`, otherwise the
published port goes nowhere.
