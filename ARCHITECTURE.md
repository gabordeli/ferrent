# Ferrent – architektúra

> **Ferrent** = *ferrous* + *torrent*. Deluge-utód Rustban: daemon + natív thin client.
> Ez a dokumentum a 2026-11-19-i tervezési beszélgetés alapján készült implementációs handoff.
> A megvalósítás sorrendje: [ROADMAP.md](ROADMAP.md).

## 1. Cél és követelmények

A Ferrent modern, alacsony erőforrás-igényű, nagy skálázhatóságú BitTorrent kliens, amely a Deluge **daemon + natív thin client** architektúráját gondolja újra Rustban.

- **Elsődleges célplatform:** Linux-first. A daemon Debian-alapú szerverekre készül (headless, 24/7); a thin client Linux-first, de Windows és macOS is cél.
- **Kemény követelmény:** legalább **4 000 egyidejű torrent** (aktív és seedelő) akadásmentes kezelése alacsony RAM- és CPU-használat mellett, a GUI és a hálózati réteg blokkolása nélkül.

| ID | Követelmény | Mérés |
|----|-------------|-------|
| R1 | 4 000+ torrent egy daemonon, újraindítás után recheck nélkül | M0 load harness, M5 soak |
| R2 | A vezérlő API és a GUI nem blokkolja a torrent motort | M1/M2 benchmark |
| R3 | Daemon RSS ≤ 1 GB 4 000 torrentnél; kliens ≤ 150 MB (natív) | harness + `ps` |
| R4 | Linux daemon (Debian/Ubuntu, x86_64 és arm64); kliens Linux/Windows/macOS | CI matrix |
| R5 | Meglévő Deluge állapot migrálható | M4 `import-deluge` |
| R6 | Sonarr/Radarr/Prowlarr/Autobrr kompatibilitás | M4 API-emuláció |
| R7 | Biztonságos távoli elérés (TLS + token) | M1 security teszt |
| R8 | Disztró-független, statikusan linkelt daemon csomag | M3 build teszt |

**Nem célok v1.0-ig:**

- Saját torrent motor implementálása (a `libtorrent` marad a motor).
- Deluge pluginok bináris/source kompatibilitása (csak migráció és API-kompatibilitás).
- Mobil kliens, média-lejátszó, streaming.
- Windows/macOS daemon.
- Több daemon egy klaszterben (horizontális skálázás).

## 2. Kulcsdöntések

| Terület | Döntés | Indok | Státusz |
|---------|--------|-------|---------|
| Motor | C++ `libtorrent-rasterbar`, `cxx` crate-tel becsomagolva | 20 éves érettség: fájlleíró-pool (`file_pool_size`), announce-ütemezés, pontos tracker-statisztika, `private=1` kezelés | rögzített |
| Motor absztrakció | `TorrentEngine` trait; a `libtorrent` az első implementáció | Ha a `librqbit` beérik, a daemon/GUI újraírása nélkül cserélhető | rögzített |
| libtorrent ág | `1.2.x` vagy `2.0.x` | 4 000+ torrentnél a memóriakezelés és az API-surface számít; mérni kell | **nyitott – M0 dönti el** |
| Daemon runtime | `tokio` | A Deluge Twistedjének megfelelője: hálózat, RPC, időzítők, háttérfeladatok | rögzített |
| Daemon–kliens protokoll | gRPC (`tonic` + Protobuf) | Típusos szerződés, streaming, codegen; alternatíva: WebSocket + bináris (`postcard`/MessagePack) | rögzített (alternatíva nyitott) |
| Perzisztencia | Beágyazott DB (`redb` vagy SQLite) | 4 000 külön `.fastresume` fájl helyett; dirty tracking, gyors indulás | **nyitott – M1 dönti el** |
| GUI | `iced` (alapértelmezett) vagy `Tauri` | `iced`: 30–50 MB RAM, natív; `Tauri`: egy frontend kódból WebUI is | **nyitott – M2 spike dönti el** |
| Repó | Egy monorepo, Cargo Workspace | Egy commit = egy funkció minden rétegben; közös protokoll típusbiztonság; külön CI jobok crate-enként | rögzített |
| Név/használat | `ferrentd`, `ferrent-cli`, `ferrent-gui` | Terminálbarát, `systemctl status ferrentd` jól néz ki | rögzített |

## 3. Repository és workspace

```text
ferrent/
├── Cargo.toml                  # Workspace gyökér (közös verziók, lintek)
├── ARCHITECTURE.md
├── ROADMAP.md
├── crates/
│   ├── protocol/               # Közös gRPC (.proto) / API típusok és szerződés
│   ├── engine/                 # TorrentEngine trait + libtorrent (cxx) implementáció
│   ├── daemon/                 # `ferrentd` bináris (tokio, state mirror, persistence, API)
│   ├── cli/                    # `ferrent-cli` terminálos vezérlő kliens
│   └── gui/                    # `ferrent-gui` natív thin client
├── packaging/
│   ├── debian/                 # systemd unit, alap config.toml, cargo-deb assetek
│   └── docker/                 # Dockerfile a headless daemonhoz
├── docs/
│   └── adr/                    # Architecture Decision Records (mért döntések)
└── .github/workflows/          # Csomagonkénti tesztek, static build, APT/Docker release
```

**Függőségi szabályok:**

- `gui` és `cli` **soha** nem függ a `engine`-től vagy C++ eszközlánctól; csak a `protocol`-tól.
- `daemon` függ az `engine`-től és a `protocol`-tól; GUI-t nem húz be.
- A `protocol` mindenki által használt, önállóan verziózott szerződés.

## 4. Komponensek

### 4.1 `engine` – torrent motor

- **Absztrakció:** tiszta Rust `TorrentEngine` trait: `add_torrent`, `pause`, `resume`, `remove`, `get_settings`, `poll_events`/alert-stream, státusz-lekérdezés.
- **Első implementáció:** `libtorrent-rasterbar` + `cxx`.
  - A `libtorrent` session **dedikált háttérszálon** fut; a `tokio` worker szálakat soha nem blokkolja.
  - Az alert-loop csak a változásokat (deltákat) tolja át egy `tokio::sync::mpsc` csatornán a daemonnak.
- **Build:** a `libtorrent` (lehetőleg Boost/OpenSSL is) **statikusan linkelve** a daemon binárisba; a `.deb`-et régebbi Debian konténerben kell fordítani a glibc visszafelé-kompatibilitás miatt.
- **Finomhangolás:** TOML-profilok (`Default`, `Seedbox / High-Performance`, `Low-RAM NAS`) az `ltconfig` szellemében: cache, max connections, disk I/O, `file_pool_size`.

### 4.2 `daemon` – `ferrentd`

- **Runtime:** `tokio`.
- **State mirror (kritikus):**
  1. A `libtorrent` session dedikált szálon fut.
  2. Az alert/delta hurok `tokio::sync::mpsc`-en küldi a változásokat.
  3. A daemon memóriában tartja a 4 000 torrent állapottábláját.
  4. A kliensek keresés/szűrés/rendezés kéréseit **ebből a cache-ből** szolgálja ki, a `libtorrent` belső lockjainak érintése nélkül.
- **Perzisztencia és recheck-védelem:**
  - Nincs 4 000 külön `.fastresume` fájl; beágyazott DB (`redb`/SQLite) tárolja a fastresume-ot, címkéket, metaadatokat.
  - Periodikus, csak a módosult rekordokat író mentés (*dirty tracking*).
  - Tiszta `SIGTERM`/`SIGINT`: fastresume kiírás és tracker `stopped` jelzések megvárása; a systemd unitban `TimeoutStopSec=120`.
- **Indulás:** 4 000 torrent betöltése recheck nélkül, másodpercek alatt.

### 4.3 `protocol` – daemon ↔ kliens

- **Szerződés:** gRPC (`tonic` + Protobuf); a `.proto` a `protocol` crate-ben.
- **Skálázhatóság:**
  - **Server-side szűrés, rendezés, lapozás:** a kliens csak a látható viewport sorait kéri le (pl. 0–50, méret szerint rendezve, `linux` szűrővel).
  - **Delta / event streaming:** polling helyett a szerver csak a ténylegesen változó torrentek frissítéseit streameli; inaktív torrent nulla üzenet.
- **Fájlátvitel:** `.torrent` hozzáadásakor a kliens a fájl **nyers bájtjait** küldi (soha nem helyi útvonalat, mert a kliens és a daemon külön gépen lehet).
- **Biztonság:**
  - Első induláskor generált önaláírt TLS tanúsítvány; a kliens első csatlakozáskor fingerprintet rögzít (**TOFU / certificate pinning**, mint az SSH `known_hosts`).
  - API token vagy Argon2-vel hashelt jelszó; a token headerben utazik, nem query paraméterben.

### 4.4 `gui` – `ferrent-gui` thin client

- **Technológia:** natív GPU-gyorsított Rust GUI (`iced`) vagy `Tauri`. A végleges döntés az M2 spike mérése után születik.
- **Követelmény:** virtualizált lista/táblázat (csak a látható ~30–50 sor renderelése), 4 000+ sornál is 60–120 FPS.
- **RAM:** `iced` ~30–50 MB; `Tauri` ~100–180 MB (WebView).
- **Kapcsolat:** TLS pinning, mentett szerverprofilok, újracsatlakozás; a daemon oldali szűrés/rendezés használata.

### 4.5 Opcionális és ökoszisztéma funkciók (v1)

- **VPN kill-switch:** `outgoing_interfaces` / `listen_interfaces` kötés (`wg0`, `tun0`); az interfész eltűnésekor a forgalom azonnali szüneteltetése, visszatérésekor automatikus újracsatlakozás.
- **Beépített automatizáció (Deluge pluginok kiváltása):** címkék/kategóriák célmappa-kezeléssel; arány/idő alapú automatikus törlés; esemény-alapú shell hookok és JSON webhookok (`on_added`, `on_finished`).
- **Observability:** opcionális Prometheus `/metrics` végpont (Grafana).
- **Migráció:** `ferrent import-deluge` – Deluge `state` könyvtár (`.torrent` + `.fastresume` + címkék) átemelése lehetőleg recheck nélkül.
- **Kompatibilitás:** opcionális qBittorrent Web API emulációs végpont (add/list/categories) a Sonarr/Radarr/Prowlarr/Autobrr számára.
- **Nyitott gRPC API:** dokumentált, hogy külső szkriptek (Python/Rust/Go) plugin nélkül automatizálhassanak.

## 5. Futásidejű adatfolyam

```text
┌──────────────┐        gRPC (delta stream, paging)        ┌─────────────────────────────┐
│ ferrent-gui  │◄─────────────────────────────────────────►│ ferrentd (tokio)           │
│ ferrent-cli  │                                           │  ├─ state mirror (memória) │
└──────────────┘                                           │  ├─ redb/SQLite (dirty)    │
                                                           │  └─ mpsc ◄── alerts        │
                                                           └──────────────┬──────────────┘
                                                                          │ dedikált szál
                                                                   ┌──────▼──────┐
                                                                   │ libtorrent  │
                                                                   │  (cxx FFI)  │
                                                                   └─────────────┘
```

- Kliens → daemon: parancsok (add/pause/resume/remove) és lapozott lekérdezések.
- `libtorrent` → daemon: csak változások (alert/delta).
- Daemon → DB: csak dirty rekordok, periodikusan és leállításkor.

## 6. Teljesítmény-büdzsé

> Javasolt célértékek; az M0 load harness kalibrálja őket, a végleges számokat ADR rögzíti.

| Metrika | Cél | Mérés |
|---------|-----|-------|
| Daemon RSS @ 4 000 torrent (95% seed) | ≤ 1 GB | harness, 1 órás soak |
| Daemon idle CPU @ 4 000 torrent | ≤ 5% egy magon | `pidstat` |
| Indulás 4 000 torrenttel | ≤ 30 s, recheck nélkül | harness |
| Kliens RSS | ≤ 150 MB (`iced`), ≤ 250 MB (`Tauri`) | `ps` |
| Kliens görgetés 4 000 sornál | ≥ 60 FPS | frame timing |
| Delta stream | inaktív torrent 0 msg/s; aktív ~1 Hz | protocol teszt |
| API latency p95 (szűrés+rendezés, 4 000 torrent) | ≤ 50 ms | benchmark |
| `.torrent` hozzáadás válaszideje | ≤ 1 s | e2e teszt |

## 7. Biztonság

- TLS minden kliens–daemon kapcsolaton; önaláírt tanúsítvány + TOFU pinning.
- Token vagy Argon2-hash; token rotálható; ne kerüljön logba.
- systemd alatt dedikált `ferrent` rendszerfelhasználó, `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ReadWritePaths=/var/lib/ferrent`.
- A daemon alapból csak a szükséges portot figyeli; a publikus expozíció a felhasználó döntése (reverse proxy vagy közvetlen TLS).

## 8. Csomagolás és terjesztés

- **Debian/Ubuntu:** `cargo-deb`; `ferrentd` csomag systemd service-szel, `/etc/ferrent/config.toml`-lal, dedikált rendszerfelhasználóval; `ferrent-gui` csomag `.desktop` fájllal.
- **APT repository:** GitHub Actions tag-build → `cargo-deb` → `apt-ftparchive`/`reprepro` index → GPG-aláírás → GitHub Pages (vagy Cloudsmith). Modern DEB822 `.sources` fájl `Signed-By=` kulccsal (`/etc/apt/keyrings/`).
- **Docker:** headless daemon image a GHCR-re (Unraid/TrueNAS/Synology/Proxmox közönség).
- **Thin client:** `.deb`, valamint AppImage vagy Flatpak a nem Debian-alapú disztrókra.
- **Linkelés:** statikus `libtorrent` (és lehetőleg Boost/OpenSSL) → egy `.deb` megy Debian 12/13-ra és az újabb Ubuntukra.

## 9. Nyitott döntések

| # | Kérdés | Mikor döntünk | Hogyan |
|---|--------|---------------|--------|
| 1 | `libtorrent` 1.2.x vagy 2.0.x | M0 | **2.0.11** – ADR 0001, engine-szintű mérés |
| 2 | gRPC vagy WebSocket + bináris | M1 | Prototípus; ha a `tonic` codegen/streaming súrlódik, váltás |
| 3 | `redb` vagy SQLite | M1 | **redb 2.6.4** – ADR 0002 |
| 4 | `iced` vagy `Tauri` | M2 | 4 000 soros spike: RAM, FPS, fejlesztési idő; ADR |
| 5 | Licenc | M0 | MIT/Apache-2.0 dual vs. GPL-3.0; felhasználói döntés |
| 6 | Minimum Debian verzió | M3 | Debian 12 vs 11 build konténer; statikus link teszt |

## 10. Forrás

- Tervezési beszélgetés (2026-11-19): Google AI Studio prompt, Drive-id `1085hFvsjd4QtvYE3RYz70G8hOtjy0jS0`.
- A részletes, ellenőrizhető feladatlista: [ROADMAP.md](ROADMAP.md).
