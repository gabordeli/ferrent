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

## GUI-hiba: a canvas-menü átütése — javítva

A canvas-os sor-menü háttere alá szorult a sorok szövege: az iced a
canvas-frame-en belül előbb a kitöltéseket, utána a szövegeket rajzolja, így
a menü téglalapja nem takarta a mögötte lévő sor-szöveget (mérve: a menü bal
szélén a `709.bin`, `4097.bin` szöveg átütött).

A második `Geometry`-be emelés **és** a második canvas (Stack) is
átütött — a szöveg a layer szintjén később rajzolódik. A megoldás: a menü
**widget-réteg** (`table::menu_items` + `floating_menu` a Stackben), mert a
widget-primitívek sorrendje megmarad. Mérve (`/tmp/g17-menu.png`,
`/tmp/g17-columns.png`): a sor-menü és a fejléc-osztályválasztó is teljesen
takar, a szöveg a panel szélénél elvágódik; a menüpont kattintása (Szünet →
„Paused by: user” a details panelen) és az oszlop-kapcsoló is működik.

Külön, **nem a menü** hibája: a `ferrent-cli pause 8` után a torrent
`seeding` maradt (a pause reason bekerült, a motor mégsem állt meg) — a
libtorrent auto-managed torrent ismert viselkedése, külön vizsgálandó.


## Amit nem mértünk

- A sparkline valódi forgalom melletti rajzolása (a minta gyűlik, de éles
  letöltés nélkül lapos).
