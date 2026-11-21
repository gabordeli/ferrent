# M10-T4 – RSS-watcher mérés

Mérve 2026-11-21, külön daemonon (`:50162`), helyi HTTP feeddel
(`python3 -m http.server`, `127.0.0.1:8099`).

## Config

```toml
[rss]
enabled = true
poll_secs = 60            # a daemon 60 s alá nem megy

[[rss.feed]]
url = "http://127.0.0.1:8099/feed.xml"
filter = "1080p"          # regex a címre
# label = "tv"
# save_path = "/srv/media/tv"
```

A feed URL-nek http(s)-nek kell lennie, a szűrőnek fordulnia kell; üres
feed-lista `enabled = true` mellett hiba (validáció a `resolve()`-ban).

## Mérés

A feed 4 itemet tartalmazott: két `1080p` (külön .torrent enclosure-rel),
egy `720p` (ugyanaz a fájl) és egy `.torrent` nélküli `1080p` item.

| Lépés | Eredmény |
|---|---|
| Első tick (indulás után azonnal) | `GET /feed.xml 200`, `GET /album.torrent 200`, `GET /album2.torrent 200` |
| Torrentek | **2** (`src`, `src2`) — a 720p-t a szűrő zárta ki, a torrent nélkülit a link típusa |
| `ferrent-cli rss` | `added=2 seen=2` |
| Restart | `GET /feed.xml 304` (If-Modified-Since), `restored_torrents=2`, **nincs új add**, `added=2` megmarad |

A dedupe kulcs a `<guid>` (fallback: link, majd cím), a store-ban
`rss-seen:<feed>:<key>` alatt; a feed metaadatai (`etag`, `last-modified`,
`last`, `error`, `added`) szintén a redb-ben élnek, ezért a restart nem
veszíti el őket.

## Hibakezelés

- Egy feed hibája (HTTP hiba, nem parse-olható XML, letöltési hiba) csak az
  adott feed `error` mezőjébe kerül, a többi feed és a daemon fut tovább.
- A már létező torrent (ugyanaz az infohash) `AlreadyExists`-ként számít
  „látottnak”, nem hiba.

## Felület

- `GetRssStatus` RPC + `ferrent-cli rss` (táblázat vagy `--json`).
- GUI: Settings → RSS fül (feed, added, seen, last poll, error) — a feedeket
  a config-dokumentumban kell felvenni, a fül a futásállapotot mutatja.

## Tesztek

- `rss::tests`: RSS 2.0 enclosure-rel, Atom `href` linkkel, torrent-URL
  felismerés (3 teszt).
- `rss_service::tests`: a dedupe-kulcs írása/olvasása MemoryStore-on.

## Ismert korlátok

- Csak a feed **link/enclosure** alapú hozzáadás megy; a leírásban lévő
  torrent-linkeket nem böngészi.
- A `poll_secs` alsó korlátja 60 s (a fájlrendszer-watcher 1 s-ja nem áll
  rendelkezésre feedeknél).
- A feedeket a config-dokumentumban kell szerkeszteni (a GUI fül
  read-only státusz).
