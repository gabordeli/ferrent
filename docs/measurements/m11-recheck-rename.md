# M11-T4 – Force recheck és fájl-átnevezés (mérés)

Dátum: 2026-11-21. Környezet: laptop, `target/release` build, éles daemon a
`/tmp/ferrent-live` adatkönyvtáron (13 torrent, `127.0.0.1:50129`), X11 GUI.

## Mit építettünk

| Réteg | Elem |
|---|---|
| engine | `Session::force_recheck` / `Session::rename_file` (bridge), `Command::ForceRecheck` / `Command::RenameFile` |
| proto | `RecheckTorrent`, `RenameTorrentFile` (a válasz a frissített fájllista) |
| daemon | `recheck_torrent`, `rename_torrent_file` (név: nem üres, nincs `/`; index: 0..len) |
| CLI | `recheck <id>`, `rename-file <id> <index> <name>` |
| GUI | sor-menü 5 elemre bővült (`Szünet/Folytatás`, `Címke beállítása…`, `Újraellenőrzés`, `Fájl átnevezése…`, `Törlés`), rename modal fájlválasztóval |

Commitok: `ccc52f0` (engine), `b821c48` (daemon + CLI + nyelvválasztó),
`717610f` (GUI sor-menü + rename modal + tranziens státuszsor-üzenet).

## Mérés – Force recheck

| Lépés | Eredmény |
|---|---|
| Sor-menü → `Újraellenőrzés` (#8, 1,0 GiB) | RPC OK, státuszsor: „Újraellenőrzés elindítva” |
| Activity fül, daemon-események | `20:50:54 seedelés ← ellenőrzés`, `20:50:55 ellenőrzés ← seedelés` |
| Állapot a mérés után | vissza `seedelés`, 100% |

A teljes `seeding → checking → seeding` kör ~1 s alatt lefutott (1 GiB,
page cache meleg), ezért a soron csak a villanás látszik – a bizonyíték az
Activity napló, nem a pillanatkép.

## Mérés – Fájl átnevezése

| Lépés | Eredmény |
|---|---|
| Sor-menü → `Fájl átnevezése…` | modal, a fájllista betöltve, egyfájlos torrentnél automatikusan kiválasztva |
| `test-1024mb.bin` → `test-1024mb-renamed.bin` | RPC OK, státuszsor: „Fájl átnevezve” |
| Lemez (`/tmp/ferrent-live/downloads`) | `test-1024mb-renamed.bin` |
| **Daemon restart** (SIGTERM → indítás, 13 torrent, 115 ms restore) | Files fül továbbra is `test-1024mb-renamed.bin` → **az átnevezés megmarad** |
| Visszaállítás (`ferrent-cli rename-file 8 0 test-1024mb.bin`) | lemez: `test-1024mb.bin` (az éles adat az eredeti állapotában) |

## Mellékesen javítva

A státuszsor a lap-számlálót mutatta (`13 torrents (1-13)`), amit minden poll
felülírt, ezért **minden** akció-visszajelzés (pause/resume, címke, hibák)
azonnal eltűnt. Most egy tranziens notice él 8 másodpercig, utána visszatér a
számláló. A `szünet/folytatás` és a többi régi üzenet továbbra is a régi úton
megy – érdemes lehet egységesíteni.

## Ismert korlát

- Torrent-**név** átnevezése még nincs (csak fájlnév a torrenten belül).
- Az átnevezés a libtorrent `rename_file`-jára épül: a lemezen azonnal
  érvényes, a fastresume-pal együtt marad meg.
