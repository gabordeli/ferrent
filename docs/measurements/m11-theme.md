# M11-T2 – téma (sötét/világos + akcentus) mérés

Mérve 2026-11-21, élő GUI a `:50129` daemonon.

## Felépítés

- `theme::Palette` — 20 szín (háttér, panel, szövegek, border, állapotok);
  `Palette::dark()` a változatlan eredeti paletta, `Palette::light()` új.
- A widgetek accessorokon keresztül kérnek színt (`theme::text_muted()`,
  `theme::bg()`, …); a kiválasztott paletta process-globális
  (`theme::install`), ezért a váltás a következő frame-ben látszik.
- `Palette::with_accent(color)` a primary + seeding színt állítja.
- A `Cargo.toml` `[profile.release] debug = 1` miatt a bináris nagyobb
  (247 MB a debug szimbólumokkal), de a panikok stacktrace-e olvasható.

## Konfiguráció

`~/.config/ferrent/gui.toml`:

```toml
theme = "light"   # dark (default) | light
accent = "#0b62d6"  # opcionális, #RRGGBB
```

A Settings → Appearance fülön a Dark/Light gomb és 8 akcentszín-minta
(`#e5484d #f76b15 #ffb224 #46a758 #12a594 #0090ff #8e4ec6 #6e7681`)
azonnal alkalmaz és menti a fájlt.

## Kontraszt

A `text_stays_readable_in_both_themes` teszt WCAG-arányt számol:

| Paletta | text | text_secondary | text_muted | error | queued |
|---|---|---|---|---|---|
| dark | 15,7:1 | 6,2:1 | 3,9:1 | 4,9:1 | 11,3:1 |
| light | 16,0:1 | 9,4:1 | 5,4:1 | 5,6:1 | 4,6:1 |

A sötét `text_muted` szándékosan 3,9:1 (a megszokott halvány tipp-szín); a
világos téma minden szövegszerepe ≥ 4,5:1.

## Képek

- `docs/measurements/m11-theme-light.png` — világos téma, élő torrentekkel.
- `docs/measurements/m11-theme-dark.png` — sötét téma, az Appearance füllel.

A világos témában a sidebar wordmarkja szövegként jelenik meg
(`theme::is_light()`), mert a csomagolt `logo.png` fehér betűs.

## Ismert korlát

- A light paletta a kék akcentust használja alapból; a progress-sávok a
  `downloading` zöldet, a kijelölés a `deep` kéket.
- Az akcentszín a primary/seeding színt állítja; a többi állapotszín fix.
