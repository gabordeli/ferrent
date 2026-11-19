# Ferrent GUI

A `ferrent-gui` iced-alapú thin client: TLS-en csatlakozik a daemonhoz, és a
torrent listát szerveroldali lapozással jeleníti meg.

## Szerverprofil és pinning

- A profilok a `~/.config/ferrent/gui.toml` (vagy `$XDG_CONFIG_HOME`) alatt
  vannak, `0600` móddal; az API token ebben a fájlban van.
- Első csatlakozáskor a daemon certificate SHA-256 fingerprintjét mutatja.
  „Trust and connect” után eltárolja; későbbi egyezésnél azonnal csatlakozik,
  eltérésnél a várt és a bemutatott fingerprintet is kiírja.
- A kapcsolat TLS, a CA PEM a trust anchor; a token nem kerül logba.

## Tábla

- A sorok virtualizáltak: csak a viewport körüli ~20 sor rajzolódik.
- A lista szerveroldali `ListTorrents` kérésekkel jön, `limit = 100`
  lapokban; a viewport elhagyásakor a következő lap töltődik.
- A fejléc kattintható: `ID`, `NAME`, `PROGRESS`, `DOWN`, `UP` szerint
  rendez (még egyszer kattintva csökkenő). A `STATE` oszlop nem
  szerveroldalon rendezhető.
- A szűrő mező a névre szűr szerveroldalon, 150 ms debounce-szal.
- Egyszeres kattintás kijelöl; Ctrl+kattintás hozzáad/elvesz;
  Shift+kattintás tartományt jelöl ki. A kijelölés oldalak között is megmarad.
- Jobb kattintás a soron kontextusmenüt nyit (Pause/Resume, Remove).
- A jobb oldali Details panel az egy kiválasztott torrent adatait, több
  kiválasztott esetén a kiválasztottak listáját mutatja.

## `.torrent` hozzáadása

A toolbaron a `.torrent` helyi útvonala és a **daemon hostján** értelmezett
save path adható meg. A GUI a fájl nyers bájtjait küldi (`AddTorrent.metainfo`),
így távoli daemonnal is működik. Egy `.torrent` fájl az ablakra húzva is
hozzáadható; a save path ilyenkor a mező aktuális értéke.

## Műveletek

- Pause, Resume és Remove: a kijelölt sorokra azonnal, optimista módon
  frissül a tábla; a daemon hibája esetén a korábbi sorállapot visszaáll.
- Remove után a lap újratöltődik; pause/resume után a következő poll hozza
  a szerver igazságát.

## Billentyűparancsok

| Billentyű | Művelet |
|---|---|
| `Space` | kijelölt torrentek pause/resume |
| `Delete` | kijelölt torrentek remove |
| `Ctrl+F` | szűrő mező fókusz |
| `Ctrl+R` | aktuális lap frissítése |
| `Ctrl+A` | az aktuális lap összes sorának kijelölése |
| `Ctrl+O` | `.torrent` útvonal mező fókusz |
| `↑` / `↓` | kijelölés mozgatása |
| `PageUp` / `PageDown` | kb. egy viewport mozgatása |
| `Home` / `End` | első / utolsó sor |
| `Esc` | kijelölés és kontextusmenü törlése |

A text input fókusza alatt a billentyűk a mezőbe mennek; kattints a táblára
a visszaváltáshoz.

## Mérés

A `--measure` flag (a `--connect`-tel) 5 másodpercig automatikusan görget,
majd `fps`, `rss_kb`, `rows` és `visible_rows` sorokat ír és kilép. A
`crates/gui/examples/demo_server.rs` 4 000 mock torrentes daemont indít a
méréshez:

```bash
cargo run --release -p ferrent-gui --example demo_server -- \
    --torrents 4000 --port 50129 --data-dir /tmp/ferrent-demo-data
XDG_CONFIG_HOME=/tmp/ferrent-demo-home \
    cargo run --release -p ferrent-gui -- --connect --measure
```

A részletes mérés: `docs/measurements/m2-gui-table.md`.
