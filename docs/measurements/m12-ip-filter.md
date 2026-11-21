# M12-T3 – IP blocklista (eMule/PeerGuardian) mérés

Dátum: 2026-11-21. Éles példány (`/tmp/ferrent-live`, 13 torrent), `release`
build; a mérés után a config és a torrentek eredeti állapotban.

## Mit építettünk

| Réteg | Elem |
|---|---|
| parser | `crates/daemon/src/blocklist.rs`: `label:start-end`, `start - end , label`, CIDR; komment/üres sor kihagyva; hibás sor → warning; duplikátum összevonás; fordított tartomány → warning |
| engine | `Session::set_ip_filter` (libtorrent `ip_filter` + `add_rule`), `set_ip_filter(&[(u32,u32)])` a traitben |
| config | `[network] blocklist = ["path vagy http(s) URL", …]`, `blocklist_refresh_hours` (default 24; `0` = nincs időzítő) |
| daemon | induláskor szinkron betöltés (a peer-kapcsolatok előtt), `spawn_refresh` időzítő, `RefreshBlocklist` RPC, `GetStatus` mezők (`blocklist_ranges`, `blocklist_refreshed_unix`, `blocklist_error`) |
| CLI | `blocklist [--refresh]` |
| GUI | Settings → Network: „Blokklista: N tartomány” + „Blokklista frissítése” gomb |

## Mérés (élő swarm)

| Lépés | Eredmény |
|---|---|
| blocklista: `192.168.2.0/24` (a LAN seed-hálózat), daemon restart | `blocklist` → `ranges: 1`, `refreshed_unix` kitöltve, hibaüzenet nélkül |
| torrent #2 (LAN seedekre épül) | **10 seed → 0 seed / 0 peer**, forgalom 0 B/s |
| torrent #3 (publikus swarm) ugyanekkor | **68 seed / 13 peer maradt**, feltöltés 127 B/s → csak a blokkolt tartomány esett ki |
| blocklista törlése a configból + restart | `ranges: 0` |

A #2-es torrent peerjei a következő tracker-announce után térnek vissza (a
tracker a peer-listát 30 percenként adja), ezért a visszaállás nem azonnali —
a filter eltávolítása a `ranges: 0`-ból látszik.

## Megjegyzés a config-újratöltésről

A `RefreshBlocklist` a **betöltött** configot használja: ha a config-fájlt
kézzel írjuk át, a watcher következő köréig a régi lista él. A mérés ezért
restarttal dolgozott; a GUI/CLI refresh a futó configot frissíti.

## Amit nem mértünk

- `.gz` tömörített blocklista (csak sima szöveg).
- HTTP-forrás letöltése (a `ureq`-alapú ág fordul, de nem lett élesben
  letöltve).
