# ferrent-cli

A `ferrent-cli` a Ferrent daemon TLS gRPC API-ját vezérli. Minden olvasó
parancs ad `--json`-t (stabil kulcsok), az író parancsok egységes exit
kódot adnak.

## Nyelv

A help és a hibaüzenetek a GUI-val közös `ferrent-i18n` katalógusból jönnek:
alapértelmezés a **magyar**, `FERRENT_LANG=en` angolra vált. A `--json`
kimenet kulcsai nyelvtől függetlenek (gépi forma), a `--help` és a
hibaszövegek lokalizáltak; a nem fordított kulcs angolul jelenik meg.
A `crates/cli` tesztje (`every_help_string_has_a_translation`) minden
clap-szöveget számon kér a katalóguson.

## Konfiguráció

Precedencia: **flag > környezeti változó > config fájl > default**.

| Flag | Env | Config kulcs |
|---|---|---|
| `--server` | `FERRENT_SERVER` | `server` vagy egy profil neve |
| `--ca` | `FERRENT_CA` | `ca` |
| `--token` | `FERRENT_TOKEN` | `token` |
| `--config` | – | – |

A default config: `$XDG_CONFIG_HOME/ferrent/cli.toml`, vagy
`~/.config/ferrent/cli.toml` (0600, mert tokent tartalmaz). Példa több
daemonnal:

```toml
active = "media"

[[servers]]
name = "media"
server = "https://192.168.2.27:50051"
ca = "/etc/ferrent/ca.pem"
token = "..."

[[servers]]
name = "laptop"
server = "https://127.0.0.1:50051"
ca = "/home/user/ferrent-data/tls-cert.pem"
token = "..."
```

A `--server media` egy profil nevére is mutathat; a `servers use media`
beállítja az `active`-ot, így a további parancsok `--server` nélkül is azt
használják. A régi lapos `server`/`ca`/`token` kulcsok továbbra is működnek.

A default server `https://127.0.0.1:50051`. A `--ca` a daemon önaláírt
tanúsítványa (a daemon kiírja az elérési útját és a SHA-256 fingerprintet).
A token nem kerül logba.

## Parancsok

### `ferrent-cli add`

```text
$ ferrent-cli add debian-13.torrent --label movies
added torrent 7
```

A `.torrent` nyers bájtjait küldi; a `--save-path` a daemon hostján
értelmezett útvonal, `--label` a címke (a staging mappája fogadja a
letöltést), `--sequential` sorrendben tölt. `--json`: `{"id", "duplicate"}`.

### `ferrent-cli list`

```text
$ ferrent-cli list --filter deb --limit 2
#  NÉV            CÍMKE   MÉRET   MENTÉSI ÚT            ÁLLAPOT       HALADÁS
7  debian-13.iso  movies  1.2 GiB /mnt/usbhdd1/library   szüneteltetve 100%
```

Szerveroldali szűrés/rendezés/lapozás; `--limit 0` = nincs limit;
`--label <name>` címkére szűr; `--sort id|name|progress|download|upload|label`.
`--json`: `{"total", "torrents": [<torrent>]}`.

### `ferrent-cli show <id>`

```text
$ ferrent-cli show 7
id:          7
name:        debian-13.iso
...
```

Egy torrent teljes pillanatképe. `--json`: `<torrent>`.

### `ferrent-cli peers <id>`

```text
$ ferrent-cli peers 7
1.2.3.4:51413            qBittorrent 4.6.2     42.1%  1.2 MiB/s down  30.0 KiB/s up
```

A csatlakozott peer-ek (végpont, kliens, haladás, sebességek). Üres listánál
`no connected peers`. `--json`: `{"id", "peers": [{"endpoint", "client",
"progress", "download_rate", "upload_rate"}]}`.

### `ferrent-cli status`

```text
$ ferrent-cli status
server:      https://192.168.2.27:50051
daemon:      ferrent 1.0.0-rc1
uptime:      52m 16s
torrents:    918 (0 active, 918 paused, 0 error)
rate:        0 B/s down / 0 B/s up
free space:  1.4 TiB
tls sha256:  ba:78:16:bf:...
```

Egy kérésből jövő egészség-összegzés: verzió, uptime, torrentszám
összes/aktív/szüneteltetett/hibás, session-sebesség, szabad hely és a pinelt
CA SHA-256 fingerprintje. A Docker healthcheck ezt hívja. `--json`: a fenti
mezők `{"server", "daemon_version", "uptime_secs", "torrent_count",
"active_count", "downloading_count", "seeding_count", "paused_count",
"error_count", "free_space_bytes", "download_rate", "upload_rate",
"fingerprint"}`.

### `ferrent-cli stats`

```text
$ ferrent-cli stats
downloaded: 1.5 TiB
uploaded:   1.4 TiB
ratio:      0.915
torrents:   918
peers:      0
uptime:     3136s
```

Session-totálok; a számlálók a daemon indulásától élnek, a torrentek
`downloaded_bytes`/`uploaded_bytes` mezőiből összegződnek.

### `ferrent-cli files <id> [--set <spec>]`

```text
$ ferrent-cli files 7 --set 0=skip,2=high
0  skip    debian-13.iso
1  normal  README.txt
2  high    extra/packages.tar
```

A fájllista és a prioritások; a `--set` `index=level` párokat vár
(`skip|low|normal|high` vagy `0-7`), a nem említett fájl `normal` marad.

### `ferrent-cli trackers <id> [--add <url>] [--remove <url>]`

```text
$ ferrent-cli trackers 7 --add udp://tracker.example:6969
udp://tracker.example:6969  working  12 seeds  4 leechers
```

Egy URL-t ír a listába (http/https/udp/ws, host kötelező), majd kiírja a
friss listát. `--json`: `{"url", "status", "seeds", "leechers", "message",
"next_announce", "fails"}` tömb.

### `ferrent-cli webseeds <id> [--set <url,url>]`

```text
$ ferrent-cli webseeds 7 --set https://mirror.example/debian-13.iso
https://mirror.example/debian-13.iso
```

A web seed lista; a `--set` a teljes listát cseréli (üres érték töröl), és a
végrehajtott listát írja vissza. `--json`: `{"id", "web_seeds"}`.

### `ferrent-cli labels`

```text
$ ferrent-cli labels
movies              3  #3ba55d  move /mnt/usbhdd1/library
music               0
```

Címkék a hozzárendelések számával (`--json`: `{"labels": [...]}`).
Alparancsok:

- `labels create <name> [--color #RRGGBB] [--move-completed <path>]
  [--download-limit N] [--upload-limit N] [--connections N]
  [--upload-slots N] [--match-tracker <s>]... [--match-name <regex>]`
- `labels update <name> ...` – csak a megadott mezőket írja át
- `labels delete <name>` – a hozzárendeléseket is törli
- `labels set <id> <name>` – torrent címkéje; üres név törli

### `ferrent-cli limits <id>`

```text
$ ferrent-cli limits 7 --download 5242880 --upload 1048576
torrent 7: limits set
```

Torrentenkénti limitek bájt/másodpercben, `--connections`, `--upload-slots`;
a `0` a címke/session szabályát örökli, `--clear` törli a kézi felülírást.

### `ferrent-cli set <id>`

```text
$ ferrent-cli set 7 --sequential --no-super-seeding
torrent 7: sequential on, super seeding off
```

`--sequential` / `--no-sequential`, `--super-seeding` / `--no-super-seeding`;
legalább az egyik pár kell. `--json`: `{"ok", "id", "set": {...}}`.

### `ferrent-cli pause <id>` / `ferrent-cli resume <id>`

```text
$ ferrent-cli pause 7
paused 7
```

A pause tartós: a daemon a `paused` flaget állítja (nem `auto_managed`-et), a
mirror azonnal frissül. `--json`: `{"ok": true, "id": 7}`.

### `ferrent-cli remove <id> [--delete-files]`

```text
$ ferrent-cli remove 7 --delete-files
removed 7
```

A `--delete-files` a letöltött adatot is törli a mentési útból.

### `ferrent-cli move <id> [--down]`

```text
$ ferrent-cli move 7
moved 7 up
```

A letöltési sorban mozgat (alap: előre).

### `ferrent-cli recheck <id>`

```text
$ ferrent-cli recheck 7
recheck requested for torrent 7
```

A meglévő adatot a hash-ekkel ellenőrzi.

### `ferrent-cli rename-file <id> <index> <name>`

```text
$ ferrent-cli rename-file 7 1 README.md
```

Egy fájl átnevezése a torrenten belül; a név nem tartalmazhat `/`-t.

### `ferrent-cli create <path>`

```text
$ ferrent-cli create /mnt/usbhdd1/library/debian-13.iso --tracker udp://t.example:6969 --output debian-13.torrent
debian-13.torrent (28416 bytes, 3f2a…)
```

A daemon hostján lévő fájlból/könyvtárból épít `.torrent`-et; a `--output`
alapja `<név>.torrent` a kliens gépén.

### `ferrent-cli rss [--preview <url>] [--filter <regex>]`

```text
$ ferrent-cli rss
example feed  enabled  every 15m  3 matched  0 downloaded

$ ferrent-cli rss --preview https://feed.example/rss --filter '1080p'
2 items, 1 matched
  yes  Some.Release.1080p
  no   Some.Release.720p  (filter)
```

A figyelő állapota; `--preview` a daemon user agentjével és időkorlátjával
letölti a feedet, és megmutatja, mit fogadna el.

### `ferrent-cli autoadd [--scan] [--path <dir>]`

```text
$ ferrent-cli autoadd
/mnt/usbhdd3/incoming  movies  pending 2  added 14  duplicates 1
$ ferrent-cli autoadd --scan
added 2, duplicates 0, deferred 0, invalid 0, failed 0
```

A figyelt mappák és a vizsgáló számlálói; a `--scan` azonnal vizsgál (a
megadott útvonalon vagy minden engedélyezett mappában).

### `ferrent-cli hooks [--limit N]`

```text
$ ferrent-cli hooks --limit 5
pending events: 0
finished  debian-13.iso  /etc/ferrent/hooks/notify.sh  exit 0  120ms
```

A hook-napló: függő események és a legutóbbi futások (parancs, exit kód,
időtartam, stderr).

### `ferrent-cli blocklist [--refresh]`

```text
$ ferrent-cli blocklist
ranges: 184213
refreshed_unix: 1795392124
```

Az IP-blokklista állapota; a `--refresh` újratölti a `[network] blocklist`
forrásait (path vagy http(s) URL, eMule/PeerGuardian vagy CIDR sorok).

### `ferrent-cli disks`

```text
$ ferrent-cli disks
/mnt/usbhdd1  ext4  /dev/sdb1  1.2 TiB free / 5.5 TiB  42 torrents
  library /mnt/usbhdd1/library
```

A tároló payload: fájlrendszerek, zónák (staging/library) és szabad hely.

### `ferrent-cli config get`

```text
$ ferrent-cli config get
[general]
data_dir = "/var/lib/ferrent"
```

A daemon config TOML dokumentuma (`--json`: `{"revision", "env_locked",
"toml"}`).

### `ferrent-cli config set <file> [--revision N]`

```text
$ ferrent-cli config set /tmp/config.toml
config updated to revision 8
```

A teljes TOML-t feltölti; `--revision N` nélkül a CLI lekéri a aktuális
revíziót. Elavult revízióra a daemon `ABORTED`-del válaszol, a futó config
nem változik.

### `ferrent-cli notify test` / `ferrent-cli notify status`

```text
$ ferrent-cli notify status
pending 0 dead 0 channels 1
```

Próbaküldés minden engedélyezett webhookra, illetve a kimenő sor számlálói.

### `ferrent-cli servers`

```text
$ ferrent-cli servers
* media        https://192.168.2.27:50051  token
  laptop       https://127.0.0.1:50051

$ ferrent-cli servers add lab --server https://192.168.2.28:50051 --ca /etc/ferrent/ca.pem --use
saved lab -> https://192.168.2.28:50051 (active) (/home/user/.config/ferrent/cli.toml)
```

A mentett daemonok a `cli.toml`-ban; `servers add <name> --server <url>
[--ca] [--token] [--use]`, `servers remove <name>`, `servers use <name>`.
A tokent soha nem írja ki, csak azt, hogy be van állítva.

### `ferrent-cli import-deluge --state-dir <dir>`

```text
$ ferrent-cli import-deluge --state-dir /var/lib/deluged/config/state --dry-run
name                     size      label   path
debian-13.iso            1.2 GiB   movies  /mnt/usbhdd1/library
```

Deluge `state/` könyvtár importja a daemon `ImportState` RPC-jén (ugyanaz a
scan, mint a GUI Import oldalán): a hiányzó címkéket létrehozza, a
torrenteket **paused** adja hozzá, info-hash szerint dedupál, és torrentenként
adja vissza az eredményt. `--dry-run` a daemon nélkül is működik, és
kilistázza a plugin-configokat is. **Éles próbánál csak publikus trackeres
torrentekkel futtasd**: a resume után az add announce-ol, és a privát tracker
kitilthat.

### `ferrent-cli watch [--count N]`

```text
$ ferrent-cli watch --count 2
{"event":"added","torrent":{...}}
{"event":"state_changed","id":7,"state":"downloading"}
```

Delta-stream; `Ctrl-C` tisztán leáll (0 exit), `--count N` N esemény után
kilép.

## RPC-lefedettség

Minden daemon RPC-nek van CLI-megfelelője; a két kivétel szándékos:

| RPC | CLI | Miért |
|---|---|---|
| `ImportPreview` | `import-deluge --dry-run` (helyi scan) | hogy a dry run daemon nélkül is működjön; az `ImportState` a daemon oldali út |
| `GetStatus` | `status`, `blocklist` | a `blocklist` a státuszból olvassa a tartomány-számot `--refresh` nélkül |

A többi RPC egy-az-egyben megfelel: `AddTorrent`→`add`, `ListTorrents`→`list`,
`GetTorrent`→`show`, `GetTorrentPeers`→`peers`, `GetTorrentFiles`→`files`,
`SetFilePriorities`→`files --set`, `GetTorrentTrackers`/`AddTracker`/
`RemoveTracker`→`trackers`, `GetWebSeeds`/`SetWebSeeds`→`webseeds`,
`ListLabels`/`CreateLabel`/`UpdateLabel`/`DeleteLabel`/`SetTorrentLabel`→
`labels`, `SetTorrentLimits`→`limits`, `SetSequential`/`SetSuperSeeding`→
`set`, `GetAutoAddStatus`/`ScanAutoAdd`→`autoadd`, `GetHookStatus`→`hooks`,
`GetNotificationStatus`/`SendTestNotification`→`notify`, `GetStats`→`stats`,
`GetDisks`→`disks`, `Watch`→`watch`, a többi torrent-művelet (`Pause`,
`Resume`, `Remove`, `Move`, `Recheck`, `RenameTorrentFile`) az azonos nevű
paranccsal.

## JSON kimenet (stabil, v1)

- `add`: `{"id": 0, "duplicate": false}`
- `list`: `{"total": 1, "torrents": [<torrent>]}`
- `show`: `<torrent>`
- `status`: lásd a `status` szakaszt
- `pause` / `resume` / `remove` / `move`: `{"ok": true, "id": 0}`
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

## Man oldal

A csomag a `ferrent-cli.1` man oldalt is telepíti
(`packaging/debian/ferrent-cli.1`); a parancslista a `--help`-fel egyezik
(a `crates/cli` teszt ellenőrzi).
