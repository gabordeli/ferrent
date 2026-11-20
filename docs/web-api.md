# Ferrent web API (v0.5)

A `ferrent-web` gateway REST felülete a daemon gRPC-ját tükrözi. Minden
`/api/*` végpont session cookie-t kér (kivéve a login), a böngésző sosem látja
a daemon tokent. TLS-t a reverse proxy ad; a gateway loopbacken figyel.

## Hitelesítés

| Metódus | Útvonal | Törzs | Válasz |
|---|---|---|---|
| POST | `/api/login` | `{"password": "…"}` | `200 {"read_only":bool,"ttl":sec}` + `Set-Cookie: ferrent_session=…; HttpOnly; SameSite=Strict` |
| POST | `/api/logout` | – | `200` + kiléptető cookie |
| GET | `/api/session` | – | `200 {"read_only":bool,"daemon":bool,"sessions":n}` |
| GET | `/health` | – | `200/503 {"status":"ok"\|"degraded","daemon":bool,"read_only":bool}` |

- **5 login kísérlet / perc / kliens** (`X-Forwarded-For` első értéke, különben
  a közvetlen cím); a hatodik `429`.
- A session memóriában él, a gateway restartja kijelentkeztet.
- `read_only = true` mellett minden mutáció `403`.

## Torrentek

| Metódus | Útvonal | Leírás |
|---|---|---|
| GET | `/api/torrents` | lapozott lista; query: `filter`, `state`, `label`, `sort`, `desc`, `offset`, `limit` (max 1000, alap 100) |
| GET | `/api/torrents/{id}` | egy torrent |
| GET | `/api/torrents/{id}/files` | `{"files":[{"path","size","progress","priority"}]}` |
| POST | `/api/torrents` | hozzáadás; `201 {"id"}`; duplicate → `409 {"id","duplicate":true}` |
| POST | `/api/torrents/{id}/pause` | `200 {"id","paused":true}` |
| POST | `/api/torrents/{id}/resume` | `200 {"id","paused":false}` |
| DELETE | `/api/torrents/{id}?delete_files=true` | `200 {"id","removed":true}` |
| PUT | `/api/torrents/{id}/label` | `{"label":"movies"}` (üres = levétel) → `{"id","label"}` |
| POST | `/api/torrents/bulk` | `{"action":"pause"\|"resume"\|"remove","ids":[…]}` → `{"ok":[…],"failed":[{"id","error"}]}` |

**Add törzs:** `magnet` (string) **vagy** `metainfo` (base64 `.torrent`);
opcionális `name`, `save_path`, `label`, `total_bytes`, `paused`. Ha a
`save_path` üres és van `label`, a daemon a címke zónájának stagingjére tölt.

**Lista válasz:** `{"torrents":[…],"total":n,"limit":n}`. A `total` a szűrés
utáni összes elem, lapozás előtt.

**Torrent objektum:** `id, name, total_bytes, state, progress, download_rate,
upload_rate, seeds, peers, eta_seconds, save_path, label, error,
queue_position`.

**`state` szűrő:** `queued, checking, downloading, seeding, paused, error,
moving` (üres/`all` = mind). **`sort`:** `id, name, progress, download_rate,
upload_rate, queue, label`.

## Egyéb

| Metódus | Útvonal | Leírás |
|---|---|---|
| GET | `/api/status` | `{"torrents","download_rate","upload_rate","free_space_bytes","states":{…},"scheduler":{…}}` |
| GET | `/api/labels` | `{"labels":[{"name","color","move_completed_path","download_rate_limit","upload_rate_limit","torrent_count"}]}` |
| POST | `/api/labels` | `{"name":"movies","color":"#2f81f7"}` → `201 {"name"}` |
| DELETE | `/api/labels/{name}` | `{"name","removed","cleared"}` |
| GET | `/api/events` | SSE: `event: deltas`, `data: {"changed":[…],"removed":[…]}` |

Az SSE a daemon `Watch` streamjét **coalescálva** adja: legfeljebb egy üzenet
a coalesce ablakban (alap 1 s), csak a változott torrentekkel, 15 s
keep-alive-pal. Reconnect után a kliens a read API-ból szinkronizál.

## Hibák

Minden hiba `{"error":"…"}` törzsű.

| HTTP | Mikor |
|---|---|
| 400 | érvénytelen query/törzs (pl. ismeretlen `state`, rossz base64) |
| 401 | nincs vagy lejárt session |
| 403 | read-only gateway mutáción |
| 404 | ismeretlen torrent id |
| 409 | duplicate add (a meglévő id-vel) |
| 412 | daemon oldali üzleti feltétel (pl. disk guard kapu) |
| 429 | login limit |
| 502/503 | daemon hiba / nincs kapcsolat |

A gRPC kódok leképezése: `NotFound→404`, `InvalidArgument/OutOfRange→400`,
`AlreadyExists→409`, `FailedPrecondition→412`, `PermissionDenied→403`,
`Unavailable→503`, `DeadlineExceeded→504`, egyéb → `502`.

## Korlátok

- Bulk: max **1000** id, részleges eredménnyel.
- A `metainfo` base64, a magnet URI változatlanul megy a daemonnak.
- Nincs CORS: a SPA azonos originon él (a reverse proxy szolgálja ki).
