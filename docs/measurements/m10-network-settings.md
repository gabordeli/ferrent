# M10-T1 – proxy, titkosítás és UPnP/NAT-PMP mérés

Mérve 2026-11-21, külön daemonon (`:50161`, `engine=libtorrent`), SOCKS5
proxyval (`/tmp/m10t1/socks5.py`, `127.0.0.1:1080`), amely minden CONNECT-et
naplóz.

## Config

```toml
[network]
proxy_type = "socks5"          # none | http | http_pw | socks5 | socks5_pw
proxy_host = "127.0.0.1"
proxy_port = 1080
proxy_peer_connections = true
encryption = "forced"           # enabled | forced | disabled
upnp = true
natpmp = true
```

Validáció (`NetworkConfig::validate`): ismeretlen `proxy_type`/`encryption`,
proxy típus host vagy port nélkül → hiba a `resolve()`-ban, a daemon nem indul
el vele. A három eset a `proxy_settings_are_validated` tesztben.

## Mérés: a forgalom tényleg a proxyn megy

A proxy naplója a daemon indulása és egy torrent hozzáadása után:

```
CONNECT 0.0.0.0:0 cmd=3            # UDP ASSOCIATE — DHT a proxyn át
CONNECT 0.0.0.0:0 cmd=3
CONNECT tracker.invalid:8000 cmd=1 # a tracker announce CONNECT-je
ERROR [Errno -2] Name or service not known
```

A `cmd=1` sor bizonyítja, hogy a HTTP announce **a proxyn keresztül** indult
(a proxy oldotta fel a nevet, ezért a hiba a proxynál keletkezett). A `cmd=3`
(UDP ASSOCIATE) azt mutatja, hogy `proxy_peer_connections = true` mellett a
DHT/UDP forgalom is a proxyhoz megy.

A daemon oldalán a tracker státusz ezt tükrözi: `error … (End of file)` — a
proxy a sikertelen feloldás után lezárta a kapcsolatot.

## Titkosítás és UPnP

- `encryption` → `settings_pack::in_enc_policy` / `out_enc_policy`
  (`pe_forced` / `pe_enabled` / `pe_disabled`); `forced` mellett a peer
  kapcsolat csak titkosított lehet.
- `upnp` / `natpmp` → `enable_upnp` / `enable_natpmp`; a default `true`, ami
  a libtorrent defaultja, tehát a config nélkül a viselkedés változatlan.
- A beállítások `apply_settings`-szel mennek a sessionbe; a proxy-kulcsok
  egy részét a libtorrent csak **új session-nel** veszi át, ezért a
  config-mentés után a daemon restartja a biztos út (a GUI is ezt írja a
  Connection fülön).

**Nem mért (nevesítve):** a `forced` mellett „csak titkosított peer" állapot
külön bizonyítása — ehhez egy csak-sima peer kellene; a policy átvezetése a
settings_packig kód-szinten történik.

## GUI

Settings → Connection fül: proxy típus/host/port/user/password, encryption,
valamint három checkbox (proxy peer connections, UPnP, NAT-PMP). A meglévő
táblás, egy mentéses mintát követi (`ConfigForm` + `set_str`/`set_bool`).

## Tesztek

- `proxy_settings_are_validated` (config parse + három hibaág),
- a meglévő `ConfigForm` roundtrip tesztek a dokumentum-írást fedik.
