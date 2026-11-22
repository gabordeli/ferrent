# Biztonsági modell és audit

Mérve 2026-11-21. A Ferrent **home-lab** daemon: egy felhasználó, egy gép,
LAN-ról elérhető RPC. Nem többfelhasználós szolgáltatás, nincs
szerepkör-alapú jogosultság.

## Vagyonek és határok

| Elem | Ki éri el | Védelem |
|---|---|---|
| `ferrentd` gRPC (`50051/tcp`) | LAN | TLS + bearer token |
| `ferrent-web` (`8443/tcp`) | LAN | TLS + ugyanaz a token |
| `/metrics` (`9898/tcp`) | opcionálisan LAN | saját, opcionális Bearer |
| qBittorrent-emuláció (`8090/tcp`) | helyi *arr | alapból **ki**, `127.0.0.1`-re köt, user/password |
| `state.redb`, letöltött adat | helyi fájlrendszer | systemd `User=ferrent`, `data_dir` jogosultság |

## Hitelesítés

- **Bearer token** minden RPC-n; az összehasonlítás
  `subtle::ConstantTimeEq`-vel történik (`crates/daemon/src/rpc/mod.rs`).
- A token a `token_file`-ból jön (csomagolva `/etc/ferrent/token`), vagy
  `FERRENT_TOKEN` env-ből; a fájlt a rendszerünk 640/600 módban tartja, a
  TLS kulcs **0600** (`rpc/tls.rs`, `web/tls.rs` — teszt is ellenőrzi).
- **Rotáció:** a token induláskor olvasódik be, tehát a fájl cseréje +
  daemon restart kell hozzá; a GUI-ban a szerverprofil tokenjét is át kell
  írni.

## TLS

- Induláskor generált, önaláírt CA + szerver-tanúsítvány; SAN: `localhost`
  és `ferrent.local` (**nincs IP SAN** — IP-vel a kliens a pinning miatt
  mégis csatlakozik, de a hostname-ellenőrzés nem megy át).
- A kliens a **tanúsítvány-ujjlenyomatot pineli** (a GUI megjegyzi a
  profillal); a CA-fájl a `data_dir`-ben.
- A `ferrent-web` ugyanígy saját tanúsítványt generál.

## Titkok

- A token, a proxy-jelszó és a Telegram bot token **nem** kerül a naplóba (a
  daemon induláskor a TLS-ujjlenyomatot és a CA-útvonalat írja, a tokent nem).
- A config-fájl írása atomikus (temp + rename) és megtartja a meglévő módot
  (alap: 0640).
- A qBittorrent-emuláció jelszava a configban **sima szöveg** — ez elfogadott,
  mert a szolgáltatás helyi, és a fájl 640.

## Amit a kód csinál, és amit nem

- **Van:** TLS minden RPC-n, token minden hívásn, state/quarantine integritás,
  a letöltött metainfo mérete korlátozott (a protobuf mérethatárán), a
  config-validáció a daemon indításakor.
- **Nincs:** több token/szerepkör, mTLS, audit-napló a hívásokról, token
  rotáció restart nélkül, rate limit az RPC-n, a trackerek HTTP-válaszának
  mélységi ellenőrzése (csak a `.torrent` feldolgozás).

## Threat model (rövid)

1. **LAN-támadó** a token birtokában: teljes kontroll a torrentek felett —
   ez a modell szerint elfogadott (a token a belépő).
2. **Rosszindulatú feed / tracker**: az RSS-watcher és a tracker-válaszok
   kívülről jönnek. Az RSS csak a konfigurált URL-eket tölti, a letöltött
   `.torrent`-et a libtorrent parse-olja; a feed szövege nem kerül shellbe
   (nincs `eval`/`sh -c` a hookokon kívül — a hookok külön, felhasználói
   konfiguráció).
3. **Hookok**: `[[hook]]` parancsot futtat; ez szándékosan tetszőleges
   parancsvégrehajtás a daemon user jogaival — a config írásjoga = kódvégrehajtás.
4. **Csomag**: a `.deb`/APT repo aláírt; a GitHub-tükör egyirányú.

## `cargo audit`

Mérve 2026-11-21, `cargo-audit 0.23.0`, 1555 advisory, 690 függőség:

- **Sebezhetőség: 0.**
- **15 figyelmeztetés** (unmaintained / unsound), a lényegesek:

| Crate | Típus | Honnan | Kezelés |
|---|---|---|---|
| `lru 0.12.5` | unsound (2 advisory) | `iced_glyphon` → iced | csak a GUI szöveg-renderelésében; iced-frissítéssel jön a javítás |
| `zvariant 4.2.0` | unmaintained | `zbus` → `dark-light` → iced | csak a GUI témaválasztásához |
| `chrono`, `chrono-tz` | unmaintained | daemon scheduler | nincs CVE; a séma stabil, a csere (jiff/time) külön döntés |
| `rustls-pemfile` | unmaintained | daemon/web TLS | kis felület; a `rustls-pki-types` kiváltja |
| `paste`, `pin-utils`, `instant`, `rustybuzz`, `ttf-parser` | unmaintained | tranzitív (iced, tonic) | nincs teendő, amíg a szülő frissül |

A `lru` „unsound” jelzései a GUI folyamatban élnek (nem a daemonban), és
`LruCache::pop()`/`IterMut` használat kell hozzá; a Ferrent nem hívja közvetlenül.

Reprodukálás: `cargo audit` (a `~/.cargo/advisory-db`-t magától frissíti).

## Hibajelentés (Bugsink)

- A `ferrentd`, `ferrent-cli` és `ferrent-gui` a Sentry SDK-val küld a
  Bugsink `ferrent` projektbe (id 5); a CLI csak a **nem várt** hibákat
  jelenti (`CliError::reportable`), a GUI és a daemon a futás végi hibát.
- **Várt feltételek** (`ferrent_telemetry::is_expected`, 2026-11-21):
  `Connection refused`, `transport error`, `connection reset`,
  `Broken pipe`, `Database already open`, `tcp connect error` — a
  forrásláncot bejárva; ezek nem lesznek issue-k.
- **redb lock:** a `RedbStore::open` 40×250 ms-ig vár a korábbi példányra
  (`waiting for the state lock (n/40): …`), nem abortál.
- **Release `debug = "line-tables-only"` + `cargo-deb strip = false`:** a
  terepen bekövetkező panic szimbólumos stacktrace-szel jön vissza. A
  `FERRENT-11` (`Option::unwrap`) azért jött `<unknown>` keretekkel, mert a
  `.deb`-et a cargo-deb alapból **strippelte**, és a SDK a folyamaton belül
  oldja fel a címeket. Az ellenőrző eszköz:
  `FERRENT_ENV=probe cargo run --release -p ferrent-telemetry --example panic-probe`
  (a `ferrent` projektbe küld egy `service=panic-probe` panicet; a kereteknek
  `fájl:sor in fn` alakúnak kell lenniük). A részletes mérés:
  `docs/measurements/telemetry.md`.
- **Config hint:** a szekcióba tartozó kulcs gyökér szinten (`temp_dir`,
  `seed_mode`, …) `unknown field` hibát ad, mostantól „belongs to the
  [session] section” útmutatással (`FERRENT-8`).
- **`unwrap`/`expect` audit:** a daemonban ~412 találat, többségük
  `Mutex`-poison `.expect`; a felhasználói bemenetre `Err`-t adunk. A
  `FERRENT-11` (`Option::unwrap`) a GUI-n kívülről, függőségből jött (a
  GUI saját kódjában nincs `unwrap`), ezért a következő előfordulás
  szimbólumos kerete mondja meg a helyet.
- Várt, lezárandó tételek: `FERRENT-10` (config hiba), `FERRENT-13`
  (`Unimplemented` — új CLI a régi daemonon).

## Nyitott pontok (M9-T2 folytatás)

- Nightly perf harness a baseline-hoz (a `ci/` mérési szkriptekre építve).
- A token rotáció restart nélkül (a `token_file` újraolvasása).
- Az RPC rate limitje (a LAN-on kívülről jelenleg nincs).
