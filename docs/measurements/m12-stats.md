# M12-T4 – Statisztika-nézet (mérés)

Dátum: 2026-11-21. Éles példány (`/tmp/ferrent-live`, 13 torrent), `release`
build.

## Mit építettünk

| Réteg | Elem |
|---|---|
| proto | `GetStats` + `GetStatsResponse` (letöltött/feltöltött bájt, arány, állapot-számlálók, peer-ek, uptime, pillanatnyi sebesség) |
| daemon | a mirror torrentjeinek összegzése (`downloaded_bytes`, `uploaded_bytes`, `peers`) + `aggregate_status` számlálók + `started_unix` |
| GUI | Details → **Statisztika** fül: számok (letöltve/feltöltve/arány, torrentek/peer-ek/üzemidő) + 5 perces sparkline (letöltés zöld, feltöltés cián); a minta 5 másodpercenként gyűlik, 60 pont |
| CLI | `ferrent-cli stats [--json]` |

## Mérés

| Lépés | Eredmény |
|---|---|
| `ferrent-cli stats` | `torrents 13`, `peers 34`, `uploaded 7.2 GiB`, `uptime 79s`, `ratio 0.000` |
| per-torrent `peers` összege (13 torrent, `show`) | **34** → a session-összeg egyezik |
| `list` darabszám | 13 → egyezik a `stats.torrents`-szel |
| daemon restart | a **session-totálok nullázódnak** (a libtorrent számlálói példányonként élnek): `downloaded 0 B`, az `uploaded` az újrakezdés óta számol |

**Miért 0 a letöltött:** a példány indítása után a torrentek seedeltek (a
payload korábban már megvolt), ezért a libtorrent session-számlálója 0-t ad;
a feltöltés 7,2 GiB a futás óta. A `ratio` ennek megfelelően 0,000.

## Ismert GUI-hiba (a felhasználói lista elé)

A canvas-os sor-menü **háttere alá szorul a sorok szövege**: az iced a
canvas-frame-en belül előbb a kitöltéseket, utána a szövegeket rajzolja, így
a menü téglalapja nem takarja a mögötte lévő sor-szöveget (mérve: a
menü bal szélén a `709.bin`, `4097.bin` szöveg átüt). A második
`Geometry`-be emelés nem oldotta meg; a tiszta javítás a menü
**widget-rétegként** (stack) való kirajzolása — a következő GUI-körre.

## Amit nem mértünk

- A sparkline valódi forgalom melletti rajzolása (a minta gyűlik, de éles
  letöltés nélkül lapos).
