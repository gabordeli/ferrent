# Fej nélküli GUI-ellenőrzés (Xvfb + xdotool)

A `ferrent-gui` (iced 0.13) ellenőrzése asztali session nélkül: Xvfb-n
futtatjuk, `import`-tal fotózunk, `xdotool`-lal kattintunk. A recept a
2026-11-23-i mérésekből jön; a csapdák mindegyike valós volt.

## 1. Xvfb és a jogosultság

```bash
Xvfb :99 -screen 0 1440x900x24 > /tmp/xvfb.log 2>&1 &
xauth -f "$HOME/.Xauthority" generate :99 ''     # enélkül: "Authorization required"
```

Ha a `$HOME` nem írható (konténer), adj meg külön célfájlt:
`xauth -f /tmp/xauth generate :99 ''`.

## 2. A GUI indítása

```bash
cd <repo>
(env -u WAYLAND_DISPLAY DISPLAY=:99 WINIT_UNIX_BACKEND=x11 \
    ICED_BACKEND=tiny-skia XDG_CONFIG_HOME=/tmp/ferrent-gui-shot \
    ./target/debug/ferrent-gui > /tmp/gui.log 2>&1 & echo $! > /tmp/gui.pid)
```

- **`ICED_BACKEND=tiny-skia`** kell: Xvfb-n nincs DRI3, a wgpu Vulkan
  panaszkodik („No DRI3 support detected") — ez warning, nem hiba.
- **`env -u WAYLAND_DISPLAY`** + `WINIT_UNIX_BACKEND=x11`: különben a
  winit a Waylandet keresi, és nem lesz ablak.
- A `XDG_CONFIG_HOME` külön könyvtárba menjen, hogy a valódi
  `~/.config/ferrent/gui.toml`-t ne írja felül az ellenőrzés.
- A daemont **előbb** indítsd (a GUI induláskor kapcsolódik; később nem
  reconnectel).

## 3. Az ablak és a kattintás koordinátái — a lényeg

Az `xdotool mousemove` **képernyő**-koordinátát vár, a widgetek viszont az
ablakon belül vannak. Az ablak pozíciója session-függő (mérve: `2584,1`),
ezért minden futásnál olvasd ki:

```bash
WID=$(DISPLAY=:99 xdotool search --name '^Ferrent$' | head -1)
DISPLAY=:99 xdotool getwindowgeometry "$WID"
#  Window 1234:  Position: 2584,1 (screen: 0)  Geometry: 1440x900
```

Kattintás = `win_x + belso_x`, `win_y + belso_y`:

```bash
DISPLAY=:99 xdotool mousemove $((WIN_X + 93)) $((WIN_Y + 805)) click 1
```

A belső koordinátát **teljes felbontású képről** mérd (lásd 4. pont), ne
tippeld: ugyanaz a sor más ablakméretnél máshol van (a sidebar
„Beállítások" sora 1440×900-on ~805, 1400×875-ön ~783).

## 4. Fotó és a nagyítás csapdája

```bash
DISPLAY=:99 import -window root /tmp/gui.png     # teljes képernyő
identify /tmp/gui.png                             # 1440x900?
convert /tmp/gui.png -crop 880x640+270+120 +repage /tmp/gui-crop.png
```

- **A kicsinyített előnézet kétszer is hazudott** (2026-11-23): egyszer
  „a téma sötét maradt" (valójában világos volt), egyszer „a gomb nem lóg
  ki" (valójában kilógott). **Mindig teljes felbontású crop** alapján
  ítélj, és a kattintási koordinátát is a cropból számold vissza.
- Az `import -window <id>` az ablakot fotózza; `-window root` a teljes
  képernyőt — utóbbi akkor jó, ha az ablak pozíciója bizonytalan.
- A képet pixelre is ellenőrizheted:
  `convert kép.png -crop 1x1+700+400 -format "%[pixel:p{0,0}]" info:`
  (világos téma: ~`srgb(240,243,246)`, sötét: ~`srgb(11,13,16)`).

## 5. Időzítés és leállítás

- Indulás után **>1 s** (iced többpasszos layoutja), a kapcsolódás és az
  első tábla ~5–7 s; a kattintás előtt ennyit várj.
- A folyamat néha magától kilép ~10–15 s után; a „terminated by signal 15"
  a **mi** `kill`ünk, nem hiba. Leállítás PID-del:

```bash
kill "$(cat /tmp/gui.pid)"
```

  **Ne** `pkill -f ferrent-gui`-t használj: a minta a saját shelled
  parancssorára is illeszkedhet, és kilövi a shellt (mérve).

## 6. Ismert buktatók (mind mérve)

| Tünet | Ok |
|---|---|
| A tábla sorai a részletek panel alatt átütnek | az iced **Canvas** rétege a kirajzolt primitívekhez nő; a rajzolónak `bounds.height`-nél meg kell állnia (`table.rs`) |
| A modal „loading…"-ot mutat placeholderekkel | a Settings négy RPC-je egymás után futott (~1,45 s); párhuzamosítva ~0,2 s (`2ec97af`) |
| A letiltott gomb aktívnak látszik | az `action_button(enabled=false)` nem kapott stílust; most szürke |
| Gépelés közben eltűnik a szövegmező tartalma | a widgetfa változik gépelés közben (blokk jelenik meg) — stabil `text_input` ID kell |
| „Authorization required" az `import`-nál | `xauth -f … generate :99 ''` az indítás előtt |

## 7. Kész példa (2026-11-23)

```bash
WID=$(DISPLAY=:99 xdotool search --name '^Ferrent$' | head -1)
eval "$(DISPLAY=:99 xdotool getwindowgeometry --shell "$WID")"   # X, Y, WIDTH, HEIGHT
DISPLAY=:99 xdotool mousemove $((X + 93)) $((Y + 805)) click 1   # Beállítások
sleep 1
DISPLAY=:99 import -window root /tmp/settings.png
kill "$(cat /tmp/gui.pid)"
```

A `getwindowgeometry --shell` `X=`, `Y=`, `WIDTH=`, `HEIGHT=` változókat
ad, így a képlet minden futásnál helyes marad.
