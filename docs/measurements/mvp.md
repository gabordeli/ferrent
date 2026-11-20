# MVP (v0.1.0) – end-to-end mérés

Mérve 2026-11-19, Debian 13.7, Intel Iris Xe, 4 000 torrentes
libtorrent 2.0.11 daemon; a 24 órás soakot 2026-11-20-án **10 000 torrenten**
megismételtük (lásd lent).

## R1 – 4 000 torrent, restart recheck nélkül

| Metrika | Érték | Budget |
|---|---:|---:|
| Betöltött torrent | 4 000 | ≥ 4 000 |
| Fastresume a leállításkor | 4 000 / 4 000 | – |
| Indulás (restart) | 6,09 s (másik futás: 4,46 s) | ≤ 30 s |
| Állapot közvetlenül indulás után | 4 000 seeding, 0 checking | recheck nélkül |
| Adat | 4 000 × 256 B payload, seed-mode prepare | – |

A `ferrentd` a redb-ből betölti a metainfót és a fastresume blobot, majd a
`LibtorrentEngine` `restore_torrent` útvonalán visszaadja a sessionnek. A
SIGTERM-es leállás 30 s-os gyűjtési ablakkal menti a resume adatokat
(`saved fastresume for 4000 of 4000 torrents`).

Reprodukció:

```bash
cargo run --release -p ferrent-daemon --features libtorrent \
  --example mvp_prepare -- --torrents 4000 --data-dir /tmp/ferrent-mvp-ready
FERRENT_DATA_DIR=/tmp/ferrent-mvp-ready FERRENT_TOKEN=secret \
  FERRENT_BIND=0.0.0.0:50133 ./target/release/ferrentd
```

## R2 – API / GUI nem blokkolja a motort

A libtorrent saját worker száron fut; a gRPC query az `ArcSwap` state
mirrorból olvas. 300 `ListTorrents` kérés, 4 000 torrent, szűrés
`payload-`, rendezés név szerint, `limit=50`:

| Metrika | Érték | Budget |
|---|---:|---:|
| RPC p50 | 44,12 ms | – |
| RPC p95 | **48,12 ms** | ≤ 50 ms |
| RPC p99 | 48,33 ms | – |
| Mirror query p95 (M1-T2) | 3,1 ms | ≤ 50 ms |

A mérés a 4 000 torrent betöltése után, seeding állapotban készült. A
szűrés+rendezés a mirror snapshoton történik; a GUI 1 s-os pollja és a
4 000 soros tábla nem nyúl a libtorrent session-höz.

Reprodukció:

```bash
./target/release/examples/rpc_bench --server https://localhost:50133 \
  --ca /tmp/ferrent-mvp-ready/tls-cert.pem --token secret \
  --queries 300 --filter payload- --limit 50
```

## R3 – memória

| Szereplő | RSS | Budget |
|---|---:|---:|
| `ferrentd` @ 4 000 torrent | 69,3 MB | ≤ 1 024 MB |
| `ferrent-gui` (iced, 4 000 sor) | 97,9 MB | ≤ 150 MB |

A kliens érték a M2-T3 4 000 soros méréséből származik
(`docs/measurements/m2-gui-table.md`). A daemon érték a restart utáni
stabil állapotból (`/proc/<pid>/status`).

## 24 órás soak

### 10 000 torrentes futás (2026-11-19 18:58 → 2026-11-20 18:58)

A végleges mérés a **10 000 torrentes** daemonon futott, 60 s-os
mintavétellel (`ci/soak.sh`), commit `8639e0b` (active_limit 20 000). A nyers
idősor: `/tmp/ferrent-soak-10k.csv` (1 440 minta + záró sor).

| Metrika | Érték | Kapu | Eredmény |
|---|---:|---|---|
| Futásidő | 24 óra 14 s, 1 440 minta | 24 óra | ✅ |
| Crash | 0 (`soak complete` sor) | nincs crash | ✅ |
| RSS kezdet → vég | 133,7 → 152,9 MB (**+14,3%**) | < 10% | ❌ |
| RSS csúcs | 160,2 MB (+19,8%) | ≤ 1 GB | ✅ |
| RSS plateau | 2026-11-20 03:53-tól ~153 MB, 15 órán át lapos | — | ✅ (nem szivárgás) |
| CPU medián | 6,7% (csúcs 71,1%, induláskor) | — | ✅ |
| API-válasz | 144/144 tízperces minta, `total=10000` | ≥ 10 percenként | ✅ |
| `list` RPC | medián 97,5 ms, p95 150 ms, max 224 ms | — | ✅ |
| `state.redb` / letöltések | 33 MB / 40 MB (10 000 fájl) | — | ~13,7 KB/torrent |

**Értékelés:** a futás crash nélkül végigment, az API minden mintában
válaszolt, és az RSS a 9. óra után **platón állt** — a +14,3% egyszeri
növekedés (state + libtorrent handle-ek), nem folyamatos szivárgás. A
szigorú `<10%` kapu ettől még **piros**; elfogadásáról külön döntés kell.

A korábbi, 4 000 torrentes (félbehagyott) futás referenciaként:

| Idő | RSS | Megjegyzés |
|---|---:|---|
| 2026-11-19 16:00 | 69 496 KB | baseline |
| 2026-11-19 16:04 | 69 716 KB | +0,3% |

A mintavevő: `ci/soak.sh`.

## Távoli GUI

A GUI egy `.27` Docker hoston futó ideiglenes Debian trixie konténerben,
Xvfb alatt, és a laptopon futó daemonhoz csatlakozott TLS-en:

- cím: `https://ferrent.local:50133` (a tanúsítvány SAN-je; a konténer
  `/etc/hosts` bejegyzése a laptop IP-jére mutat),
- 4 000 torrent, `connected`, első oldal betöltve,
- screenshot: `docs/measurements/m2-remote-gui.png`.

Ez igazolja, hogy a kliens nem-localhost TLS útvonalon is működik. A
tanúsítvány csak `localhost`/`ferrent.local` SAN-t tartalmaz, ezért IP-ről
a hostname-ellenőrzés elbukik; ez ismert korlát.

## Ismert korlátok

- A mérés seed-mode prepare-rel készült (4 000 × 256 B); nincs benne valós
  peer-forgalom, tracker announce vagy részleges letöltés.
- A soak egyetlen daemon, egyetlen kliens nélküli idle seeding terhelés.
- A GUI `STATE` oszlopa nem rendezhető szerveroldalon.
- A `.torrent` save path a daemon hostján értelmezett.
- A token plaintext a `0600` configban; keyring későbbi.
- A TLS tanúsítvány nem tartalmaz tetszőleges IP SAN-t.
- Nincs `.deb`/APT/Docker csomagolás, Deluge-import, `*arr` API (v1.0).
