# Telemetria és szimbolizáció (M13-T7 / #73) – mérések

Mérve 2026-11-22, Debian 13 host, `cargo` 1.100.0, `cargo-deb` 3.8.0,
Bugsink `ferrent` projekt (id 5).

## A kiinduló 5 issue

| Issue | Típus | Darab | Ok |
|---|---|---:|---|
| FERRENT-14 | panic | 1 | duplikált `STATUS` kulcs a `hu.toml`-ban (a `02fa3fc` javította) |
| FERRENT-11 | panic | 1 | `Option::unwrap()` — **szimbolizálatlan** keretek |
| FERRENT-8 | Parse | 9 | gyökér szintű `temp_dir` (a `[session]` alá tartozik) |
| FERRENT-12 | Status | 1 | `transport error (Unknown)` a CLI `Status` ágából |
| FERRENT-5 | Transport | 21 | daemon-restart zaj, a `is_expected()` előtti időkből |

## Miért voltak `<unknown>` keretek

A nyers esemény (`FERRENT-11`) csak `instruction_addr` mezőket tartalmazott,
`debug_meta` nélkül; a két libc keret neve a dinamikus szimbólumból jött.

1. A `sentry` Rust SDK a **folyamaton belül** oldja fel a címeket a bináris
   szimbólumaiból (`.symtab` + DWARF) — szerveroldali szimbolizáció nem kell.
2. A `cargo-deb` viszont **alapból strippel**, tehát a `.deb`-ből telepített
   `ferrentd`/`ferrent-gui` szimbólumok nélkül futott.

Bizonyíték: a `crates/telemetry/examples/panic-probe.rs` ugyanazzal a
profillal, nem strippelve:

```text
### panic-probe.rs:17 in `panic_probe::main` [in-app]
### option.rs:971 in `<core::option::Option<u8>>::expect` [external]
### panicking.rs:80 in `core::panic::panicking::panic_fmt` [external]
```

Ugyanez a próba a strippelt binárisból `<unknown>` kereteket ad — pontosan
azt, amit a FERRENT-11 mutatott.

**FERRENT-11 visszafejtési kísérlet (2026-11-22):** a v0.1.2 tag **release
profil nélkül** készült (`debug = 0`), tehát a `.deb`-be került bináris
szimbólum nélkül ment. Az újraépített v0.1.2 GUI-ban (30 MB, `.symtab` van,
`.debug_line` nincs) a rögzített címek egyike sem adott page-aligned bázist
egyetlen ismert szimbólumhoz sem, tehát a buildkörnyezet eltér (a pontos
bináris már nincs meg). A GUI saját kódjában nincs `unwrap`, a panic
függőségből jött; a következő, már szimbolizált előfordulás mondja meg a
helyet.

## Változás

| Beállítás | Előtte | Utána |
|---|---|---|
| `[profile.release] debug` | `1` (teljes DWARF) | `"line-tables-only"` |
| `cargo-deb strip` | alapértelmezett (**igen**) | `false` (daemon + GUI) |
| CLI `Status(Unknown)` | jelentve | `is_expected()` szűri a transport szöveget |

A `line-tables-only` a fájl:sor információt adja meg, a teljes DWARF
nélkül; a függvénynevek a `.symtab`-ból jönnek. A próba ezzel is
`panic-probe.rs:17 in main` kereteket ad (a generikus argumentumok
rövidülnek, a triázshez elég).

## Méretek

| Csomag | Strippelt (0.1.3) | Nem strippelt, `debug = 1` |
|---|---:|---:|
| `ferrent` (daemon + CLI) | 21 MB | 32,5 MB |
| `ferrent-gui` | ~15 MB | 35 MB |

A `line-tables-only` csomagmérete a következő kiadás buildjében mérhető
(a próba bináriusa 132 MB → 28 MB volt a teljes DWARF-hoz képest).

## Ellenőrzés

```bash
FERRENT_ENV=probe cargo run --release -p ferrent-telemetry --example panic-probe
```

A Bugsinkban a `service=panic-probe` tagű esemény kereteinek
`fájl:sor in függvény` alakúaknak kell lenniük. Csomagolt binárison:

```bash
dpkg-deb -x dist/ferrent_*_amd64.deb /tmp/pkg
readelf -S /tmp/pkg/usr/bin/ferrentd | grep -E '\.symtab|\.debug_line'
```

## 2026-11-24-i záró ellenőrzés

**Telepített `.deb` (a .27-en, `1.0.0~rc1-1`):** a `readelf -S
/usr/bin/ferrentd` megtalálja a `.debug_line`-t és a `.symtab`-ot, tehát a
`cargo deb --no-strip` a gyakorlatban is működik, nem csak a build szkriptben.

**AppImage (rc1):** a kicsomagolt `usr/bin/ferrent-gui`-ban **nincs**
`.symtab`/`.debug_line` — a linuxdeploy strippel, és nincs rá kapcsoló. Az
AppImage-ből indított GUI panicja tehát továbbra is `<unknown>` keretekkel
megy a Bugsinkbe; a szimbolizált út a `.deb`. Ha ez fontos, a linuxdeploy
után vissza kell másolni a nem strippelt binárist az AppDirba (a méretek
fönt: a strippelt GUI ~15 MB, a szimbólumos 35 MB — döntési pont).

**24h+ mérés:** a 2026-11-22-én resolved issue-k `count`-ja nem nőtt
(FERRENT-14: 1, FERRENT-8: 9, FERRENT-12: 1, FERRENT-5: 21) — a szűrések
után nincs új előfordulás.

**Új issue-k triázsa (2026-11-24):**

| Issue | Ok | Állapot |
|---|---|---|
| FERRENT-15 | a `panic-probe` saját eseményei (`environment=probe`, `service=panic-probe`) | resolve — ez a szimbolizáció kapuja |
| FERRENT-16 | a CLI configjában a `ca = "~/…"` **nem** expandálódott (a fájl a `$HOME` alatt volt, a literál `~` miatt mégis „no such file”) | javítva, resolve |
| FERRENT-17 | `ferrentd --token-file /tmp/pause-probe/token` — saját pause-próba, a fájl nem létezett | resolve — teszt-artefakt |

A `~` feloldása (`crates/cli/src/config.rs` és a GUI profil betöltése,
`crates/gui/src/config.rs`) csak a kezdő `~/`-t bontja ki a `$HOME`-ból, a
többi részt szó szerint hagyja; tesztek: `a_home_relative_ca_is_expanded`,
`a_home_relative_ca_is_expanded_on_load`.

**FERRENT-11 továbbra is nyitva:** a v0.1.2 szimbolizálatlan címkéit nem
lehet visszafejteni (a bináris már nincs meg), a GUI saját kódjában nincs
`unwrap`, tehát függőségből jött. Az első, már szimbolizált előfordulás
mondja meg a helyet; addig nincs mit „Option-kezelni”.
