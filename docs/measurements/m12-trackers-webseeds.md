# M12-T2 – Tracker szerkesztés és web seed lista (mérés)

Dátum: 2026-11-21. Éles példány (`/tmp/ferrent-live`, 13 torrent), `release`
build, a 8-as és 9-es torrenten mérve; a végén az eredeti állapot visszaállítva.

## Mit építettünk

| Réteg | Elem |
|---|---|
| engine | `Session::add_tracker` / `remove_tracker` / `web_seeds` / `set_web_seeds` (libtorrent `announce_entry`, `replace_trackers`, `url_seeds`) |
| proto | `AddTracker`, `RemoveTracker`, `GetWebSeeds`, `SetWebSeeds` |
| daemon | a négy handler; URL-validáció (`http/https/udp/ws` + host; web seed csak `http(s)`), üres/érvénytelen → `InvalidArgument` |
| CLI | `trackers <id> --add/--remove`, `webseeds <id> [--set a,b]` |
| GUI | Trackers fül: „Add tracker” input + soronkénti `×`, „WEB SEEDEK” szakasz ugyanígy |

Commitok: `0a1c1f1`-szerű sorozat (engine → RPC/CLI → GUI → docs).

## Mérés

| Lépés | Eredmény |
|---|---|
| `trackers 8 --add http://127.0.0.1:6969/announce` | a lista végére került, állapot `updating` (announce indul) |
| `trackers 8` a hozzáadás után | látszik a két tracker: a saját és a LAN-os (`Error`, „unreachable”) |
| `trackers 8 --add not-a-url` | `InvalidArgument`, a lista nem változik |
| **daemon restart** (13 torrent, ~4 s leállás) | a hozzáadott tracker **megmaradt** (libtorrent resume data), továbbra is announce-ol |
| `webseeds 9 --set "http://mirror.example/files/,https://mirror2.example/f"` | mindkettő megjelenik, `webseeds 9` visszaadja |
| **daemon restart** | a web seed lista **megmaradt** |
| GUI Trackers fül (#8) | a két tracker sora + `×`, alatta az „Add tracker” input és a „WEB SEEDEK” szakasz |

## Ismert korlát

- A hozzáadott tracker csak akkor „Working”, ha a tracker válaszol; a LAN
  tracker scrape nélkül `updating`/`Error` – ez nem hiba.
- A web seedek tényleges letöltése nem lett megmérve (a minta-URL-ek nem
  léteznek); a lista kezelése és a perzisztencia igen.
