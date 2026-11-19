# M2-T3 – Virtualizált torrent tábla (4 000 sor)

Mérve 2026-11-19, Debian 13.7, Intel Iris Xe, XWayland, 1100×700,
release build, 5 s automatikus görgetés.

## Eredmény

| Metrika | 1. futás | 2. futás |
|---|---:|---:|
| FPS | 59,8 | 59,4 |
| RSS | 100 132 KB ≈ 97,8 MB | 100 200 KB ≈ 97,9 MB |
| Összes sor | 4 000 | 4 000 |
| Kirajzolt sor | 20 | 20 |

A kijelző 60 Hz-es vsync; a mért 59,4–59,8 FPS a 60 Hz-es felső korlát
gyakorlati értéke. A korábbi M2-T1 spike ugyanilyen környezetben 59,0 FPS-t
mért; a tábla egy soronkénti szöveg + háttér rajzolással dolgozik, csak a
viewport 20 sorát alakítja ki.

## Hogyan

A méréshez a GUI `--measure` módja 5 másodpercig 18 px/frame-mal görget,
közben a látható tartományhoz kér szerveroldali lapot, majd kiírja az FPS-t
és az RSS-t. A daemon a `crates/gui/examples/demo_server.rs` 4 000 mock
torrenttel:

```bash
cargo build --release -p ferrent-gui --example demo_server
./target/release/examples/demo_server --torrents 4000 --port 50129 \
    --data-dir /tmp/ferrent-demo-data

XDG_CONFIG_HOME=/tmp/ferrent-demo-home \
    ./target/release/ferrent-gui --connect --measure
```

A GUI a listát `ListTorrentsRequest { limit: 100, offset: ... }`
kérésekkel tölti; görgetéskor a viewport körüli 100 soros lapot kéri le,
nem a teljes 4 000-et. A státuszsáv a lap határait mutatja
(`4000 torrents (1-100)`, `... (896-995)`).

## Képernyőképek

| Állapot | Fájl |
|---|---|
| 4 000 sor, teteje | `docs/measurements/m2-gui-table-4000.png` |
| 4 000 sor, görgetve | `docs/measurements/m2-gui-table-scrolled.png` |
| Szerveroldali szűrés | `docs/measurements/m2-gui-table-filtered.png` |
| Kis adathalmaz (kézi műveletek) | `docs/measurements/m2-gui-table.png` |

## Manuális ellenőrzések

- Szűrés: `torrent-39` → `100 matching torrents (1-100)`, csak a találatok.
- Rendezés: `NAME` fejléc → szerveroldali névsorrend, a lap újratöltődik.
- Többszörös kijelölés: Ctrl+kattintás, Shift+tartomány; a Details panel
  a kiválasztottakat mutatja.
- Kontextusmenü: jobb kattintás → Pause/Resume, Remove; a művelet a
  kijelölésre fut.
- `.torrent` hozzáadás: a GUI a helyi fájl nyers bájtjait küldi; a daemon
  a saját hostján értelmezi a save pathot.
- Optimista pause/resume/remove; hiba esetén a sorok visszaállnak, majd a
  poll a szerver állapotát hozza.
- Billentyűk: Space, Delete, Ctrl+F/R/A/O, nyilak, PageUp/PageDown,
  Home/End, Esc; dokumentálva: `docs/gui.md`.

## Korlátok

- A `STATE` oszlop nem szerveroldalon rendezhető (a protokoll
  `SortField` értékei: id, name, progress, download, upload).
- A 100 soros lap mérete fix; a viewport körüli lapot a görgetés
  pozíciója alapján kéri a kliens.
- Az add save path a daemon hostján értelmezett útvonal; a kliens nem
  ellenőrzi, hogy létezik-e.
