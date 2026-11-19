# ferrent-cli

A `ferrent-cli` a Ferrent daemon TLS gRPC API-ját vezérli.

## Konfiguráció

Precedencia: **flag > környezeti változó > config fájl > default**.

| Flag | Env | Config kulcs |
|---|---|---|
| `--server` | `FERRENT_SERVER` | `server` |
| `--ca` | `FERRENT_CA` | `ca` |
| `--token` | `FERRENT_TOKEN` | `token` |
| `--config` | – | – |

A default config: `$XDG_CONFIG_HOME/ferrent/cli.toml`, vagy
`~/.config/ferrent/cli.toml`. Példa:

```toml
server = "https://127.0.0.1:50051"
ca = "/home/user/ferrent-data/tls-cert.pem"
token = "..."
```

A default server `https://127.0.0.1:50051`. A `--ca` a daemon önaláírt
tanúsítványa (a daemon kiírja az elérési útját és a SHA-256 fingerprintet).
A token nem kerül logba.

## Parancsok

```text
ferrent-cli add <file> [--name <name>] [--save-path <path>]
ferrent-cli list [--filter <s>] [--sort id|name|progress|download|upload]
                 [--desc] [--offset N] [--limit N]
ferrent-cli show <id>
ferrent-cli pause <id>
ferrent-cli resume <id>
ferrent-cli remove <id>
ferrent-cli watch [--count N]
```

- `add`: a `.torrent` fájl nyers bájtjait küldi; a `--save-path` a daemon
  hostján értelmezett útvonal.
- `list`: szerveroldali szűrés/rendezés/lapozás; `--limit 0` = nincs limit.
- `watch`: delta-stream; `Ctrl-C` tisztán leáll; `--count N` N esemény után
  kilép.
- `--json` globális flag: minden parancs JSON-t ad.

## JSON kimenet (stabil, v1)

- `add`: `{"id": 0}`
- `list`: `{"total": 1, "torrents": [<torrent>]}`
- `show`: `<torrent>`
- `pause` / `resume` / `remove`: `{"ok": true, "id": 0}`
- `<torrent>`: `{"id", "name", "state", "progress", "total_bytes",
  "download_rate", "upload_rate", "error"}`
- `watch`: soronként egy objektum:
  - `{"event":"added","torrent":<torrent>}`
  - `{"event":"state_changed","id":0,"state":"paused"}`
  - `{"event":"progress","id":0,"progress":0.5,"download_rate":1,"upload_rate":2}`
  - `{"event":"removed","id":0}`

A `state` értékek: `unspecified`, `queued`, `checking`, `downloading`,
`seeding`, `paused`, `error`.

## Exit code-ok

| Kód | Jelentés |
|---|---|
| 0 | siker |
| 1 | általános futásidejű hiba |
| 2 | usage vagy konfigurációs hiba |
| 3 | nem található (gRPC `NotFound`) |
| 4 | hitelesítési hiba (`Unauthenticated`/`PermissionDenied`) |
| 5 | kapcsolódási/TLS hiba |

## Daemon indítása

```bash
FERRENT_TOKEN=secret \
FERRENT_DATA_DIR=./ferrent-data \
FERRENT_BIND=127.0.0.1:50051 \
cargo run -p ferrent-daemon
```

A default daemon a `MockEngine`-t használja; valódi torrentekhez:

```bash
FERRENT_TOKEN=secret FERRENT_DATA_DIR=./ferrent-data \
cargo run -p ferrent-daemon --features libtorrent
```

A daemon kiírja a cert útját és a fingerprintet; a CLI-nek a
`--ca ./ferrent-data/tls-cert.pem` és a `--token secret` kell.

A daemon a `FERRENT_DATA_DIR` alatt tartja a `state.redb`-t; a torrent
rekordok újraindítás után visszajönnek. `SIGTERM`/`SIGINT` esetén fastresume-t
ment; `kill -9` után az utolsó mentés elavulhat, ilyenkor recheck lehetséges.
