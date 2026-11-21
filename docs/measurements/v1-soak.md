# M9-T1 – Soak, crash-recovery és robustness mérés

Mérve 2026-11-21, a munkagépen (Debian, 4 mag, LVM root). A daemon
`engine=libtorrent` (release build, `cargo build --release`).

## 1. `kill -9` – éles adat, 12 torrent

A `/tmp/ferrent-live` példány (12 valódi torrent, mind seeding, 100%):

| | Érték |
|---|---|
| Előtte (CLI snapshot) | `total=12`, `states=[seeding]`, `progress=[100.0]` |
| `kill -9` | a folyamat azonnal meghalt, a port felszabadult |
| Restart | `engine=libtorrent`, `restored_torrents=12`, `startup_ms=227.1` |
| Utána (CLI snapshot) | `total=12`, `states=[seeding]`, `progress=[100.0]`, `checking=[]` |
| Azonos id-k | igen (a 12 id halmaza változatlan) |

Tehát a SIGKILL után a checkpoint/fastresume állapotból **recheck nélkül**
állt vissza, ugyanazokkal a torrentekkel. Előtte biztonsági mentés:
`/tmp/ferrent-live/state.redb.bak-kill9`. A `checkpoint_interval` 120 s, tehát
legfeljebb ~2 percnyi progressz veszhet el – itt nem is veszett, mert minden
torrent 100%-on állt.

## 2. Sérült DB rekord → karantén

Új útvonal: `persist::load_torrents_quarantining` (a daemon ezt használja
induláskor). A hibás kulcs a `quarantine:<eredeti kulcs>` alá kerül (a redb-be
írva, még a serve előtt), a jó rekordok betöltődnek; a daemon indul.

- érvénytelen UTF-8 mező → a torrent a fallback névvel betölt, a mező karanténba;
- értelmezhetetlen kulcs (`torrent:not-an-id:name`) → karantén, nem hiba;
- hiányos rekord (metainfo vagy save_path nélkül) → a rekord összes kulcsa karanténba;
- redb-reopen után a karantén megmarad, a `torrent:` alatt nem marad szemét.

Induláskor a daemon kiírja: `quarantined_records=N`, majd soronként
`quarantined=<kulcs> reason=<ok>`.

Tesztek: `persist::tests::{a_malformed_key…, an_unreadable_field…,
an_incomplete_record…, quarantine_is_written_to_redb_before_the_daemon_serves}`
(4 új, zöld).

## 3. Malformed RPC → hiba, nem panic

`crates/daemon/tests/rpc_tls.rs`:

- `garbage_on_the_tls_port_does_not_kill_the_server`: a TLS portra nyers
  `GARBAGE … HTTP/1.1` bájtok mennek (handshake nélkül); utána egy normál
  `ListTorrents` továbbra is működik.
- `an_unknown_enum_value_is_tolerated`: `sort=99`, `state_filter=0xdead`
  (nem létező enum értékek) → `Ok`, üres lista; a konverzió `try_from(..).unwrap_or(..)`.

Az egész `rpc_tls` suite 9/9 zöld. **Nem** tesztelt (nevesítve): kézzel
injektált hibás HTTP/2 frame / csonka protobuf body — azt a tonic kezeli, de
nincs rá saját teszt.

## 4. 72 órás soak – 4 000 torrenttel

Fixture: `mvp_prepare --torrents 4000` (4000 valódi, seedelt torrent, 256 B
payload; `added=4000 add_secs=2.680`, state 17 MB).

| | Érték |
|---|---|
| Indulás | 2026-11-21 10:46 (CET), `:50150`, `/tmp/ferrent-soak-4k` |
| Restore | `restored_torrents=4000`, `startup_ms=2528.9` |
| RSS baseline | 79 048 KiB (≈77 MB) |
| fd baseline | 93 |
| Elfogadási küszöb (RSS < +10%) | ≤ 86 953 KiB |
| Mintavétel | `ci/soak.sh` 60 s-enként → `/tmp/ferrent-soak-4k/soak.csv` |
| 72 h vége | 2026-11-24 ~10:47 |

Első minták: `10:47:22 rss=79 580 total=4000 list_ms=106`;
`10:48:22 rss=80 480`.

**Állapot: fut, a 72 órás ítélet még nyitott.** A záráshoz a CSV-ből kell
igazolni: nincs crash-sor, RSS a küszöb alatt marad, a `total` végig 4000,
és a fd-szám nem nő monoton.

## Ismert korlátok

- A soak a helyi gépen fut, nem a LAN teszthálón (caprice seed + tracker);
  a peer-forgalom itt minimális, a terhelés a torrent-szám és az RPC.
- A `kill -9` bizonyítás 12 torrenten történt (éles adat), nem 4000-en;
  a 4000-es restore a soak indulásakor mért 2,5 s.
- A malformed-RPC lefedettség a fenti két eset; a nyers HTTP/2 frame szint
  nincs külön tesztelve.
