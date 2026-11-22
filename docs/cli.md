# ferrent-cli

A `ferrent-cli` a Ferrent daemon TLS gRPC API-ját vezérli.

## Nyelv

A help és a hibaüzenetek a GUI-val közös `ferrent-i18n` katalógusból jönnek:
alapértelmezés a **magyar**, `FERRENT_LANG=en` angolra vált. A `--json`
kimenet kulcsai nyelvtől függetlenek (gépi forma), a `--help` és a
hibaszövegek lokalizáltak; a nem fordított kulcs angolul jelenik meg.

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
ferrent-cli config get
ferrent-cli config set <file> [--revision N]
ferrent-cli trackers <id> [--add <url>] [--remove <url>]
ferrent-cli webseeds <id> [--set <url,url>]
ferrent-cli blocklist [--refresh]
ferrent-cli stats [--json]
ferrent-cli import-deluge --state-dir <dir> [--config-dir <dir>] [--dry-run]
```

- `add`: a `.torrent` fájl nyers bájtjait küldi; a `--save-path` a daemon
  hostján értelmezett útvonal.
- `list`: szerveroldali szűrés/rendezés/lapozás; `--limit 0` = nincs limit;
  `--label <name>` labelre szűr; `--sort label` label szerint rendez.
- `watch`: delta-stream; `Ctrl-C` tisztán leáll; `--count N` N esemény után
  kilép.
- `trackers <id>`: a torrent announce-listája; `--add`/`--remove` egy URL-t
  ír a listába (http/https/udp/ws, host kötelező), majd kiírja a friss listát.
- `webseeds <id>`: a web seed (HTTP tükör) lista; `--set` a teljes listát
  cseréli (üres érték töröl). A végrehajtott listát írja vissza.
- `stats`: session-totálok (letöltött/feltöltött bájt, arány, torrentek, peer-ek, uptime); a számlálók a daemon indulásától élnek.
- `blocklist`: az IP-blokklista állapota (tartomány-szám, utolsó betöltés);
  `--refresh` újratölti a `[network] blocklist` forrásait (path vagy http(s)
  URL, eMule/PeerGuardian vagy CIDR sorok).
- `config get`: a daemon config TOML dokumentuma és a revízió; `--json`-nal
  `{"revision", "env_locked", "toml"}`.
- `config set <file>`: a teljes TOML-t feltölti; `--revision N` nélkül a CLI
  lekéri a aktuális revíziót. Elavult revízióra a daemon `ABORTED`-del válaszol,
  a futó config nem változik.
- `import-deluge`: Deluge `state/` könyvtár importja a daemon `ImportState`
  RPC-jén (ugyanaz a scan, mint a GUI Import fülén): a hiányzó címkéket
  létrehozza, a torrenteket **paused** adja hozzá, info-hash szerint dedupál,
  és torrentenként adja vissza az eredményt. `--dry-run` a daemon nélkül is
  működik, és kilistázza a plugin-configokat is. **Éles próbánál csak publikus
  trackeres torrentekkel futtasd**: a resume után az add announce-ol, és a
  privát tracker kitilthat.
- `--json` globális flag: minden parancs JSON-t ad.

## JSON kimenet (stabil, v1)

- `add`: `{"id": 0, "duplicate": false}`
- `list`: `{"total": 1, "torrents": [<torrent>]}`
- `show`: `<torrent>`
- `pause` / `resume` / `remove`: `{"ok": true, "id": 0}`
- `config get`: `{"revision": 0, "env_locked": ["bind"], "toml": "..."}`
- `config set`: `{"revision": 1}`
- `<torrent>`: `{"id", "infohash", "name", "save_path", "queue_position",
  "pause_reasons", "label", "state", "progress", "total_bytes",
  "download_rate", "upload_rate", "error"}`
- `watch`: soronként egy objektum:
  - `{"event":"added","torrent":<torrent>}`
  - `{"event":"state_changed","id":0,"state":"paused"}`
  - `{"event":"progress","id":0,"progress":0.5,"download_rate":1,"upload_rate":2}`
  - `{"event":"removed","id":0}`
  - `{"event":"finished","id":0}`
  - `{"event":"config_changed","revision":1}`

A `state` értékek: `unspecified`, `queued`, `checking`, `downloading`,
`seeding`, `paused`, `moving`, `error`.

## Exit code-ok

| Kód | Jelentés |
|---|---|
| 0 | siker |
| 1 | általános futásidejű hiba |
| 2 | usage vagy konfigurációs hiba |
| 3 | nem található (gRPC `NotFound`) |
| 4 | hitelesítési hiba (`Unauthenticated`/`PermissionDenied`) |
| 5 | kapcsolódási/TLS hiba |

Kapcsolódási hibánál az üzenet megnevezi a célt és az okot is, pl.
`ferrent-cli: cannot connect to https://127.0.0.1:50051: transport error: tcp
connect error: Connection refused (os error 111)`.

A `ferrent-cli` csak a **várt** hibákat jelenti telemetrián (Bugsink): a leállt
daemon (5), a nem létező torrent (3), a rossz token (4) és a `| head` miatti
zárt cső normális válasz, ezek nem generálnak issue-t. A konfigurációs hibák
és a szerveroldali `Internal`/`Unknown`/`DataLoss`/`Unimplemented` státuszok
mennek csak fel.

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
