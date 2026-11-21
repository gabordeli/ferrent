# Ferrent web (M7) – mérések

Mérve 2026-11-20, Debian 13, 12 szál (i7-1355U) + a laptop LAN-IP-je
(192.168.2.65). A daemon `MockEngine`-en fut (10 000 torrent), a gateway
debug buildben; a TLS a gateway saját certje (ring provider).

## M1 – Lista 10 000 torrenten (HTTPS, keep-alive)

| Metrika | Érték | Budget |
|---|---:|---:|
| `GET /api/torrents?limit=50&sort=name` p50 | **59,8 ms** | – |
| p95 | **64,3 ms** | < 300 ms |
| max (50 kérés) | 65,7 ms | – |
| szűrt lekérdezés (`filter`+`state`+`sort`) | **9,7 ms** | – |
| 200 torrenten (korábbi mérés) p50 / p95 | 52,5 / 56,6 ms | – |

A HTTPS handshake külön ~30–50 ms, ha a kliens nem tartja a kapcsolatot; a
böngésző (HTTP/1.1 keep-alive) ezt egyszer fizeti.

## M2 – SSE fan-out (3 kliens)

- 3 párhuzamos `/api/events` kliens, 30 s-enként egy add + egy remove a
  traffic-generátorból.
- Mindhárom kliens **ugyanazokat a coalescált üzeneteket** kapta (2 üzenet
  45 s alatt, a coalesce ablak 1 s).
- A gateway **egy** daemon `Watch` előfizetést tart, a fan-out a
  `broadcast` csatornán megy; lassú kliens nem blokkol.

## M3 – TLS

- A cert first startkor generálódik (SAN: `names` + `localhost` +
  `127.0.0.1`/`::1`), a kulcs **0600**, a fingerprint a startup logban.
- A session cookie HTTPS alatt **`Secure`** (`HttpOnly; SameSite=Strict`).
- A `rustls` provider: `ring` (az `axum-server` `tls-rustls-no-provider`
  feature-je + explicit `install_default`), különben a provider-választás
  Ambiguous lenne a tonic `tls-ring`-jével.

## M4 – Terhelés (10 000 torrent, 3 SSE kliens, 1 óra)

Módszer: `demo_server --torrents 10000`, gateway HTTPS-en, 3 SSE kliens +
30 s-enként egy add/remove; 60 s-enként RSS-minta és SSE-üzenetszám.

| Idő | RSS | Megjegyzés |
|---|---:|---|
| indulás után ~1 perc | **8,4 MB** | üres gyorsítótár, 10k lista már kiszolgálva |
| csúcs | **9,1 MB** | a 10k katalógus első kiszolgálása |
| 60 perc után | **7,9 MB** | nincs növekedés |

Mérve 2026-11-20 22:13:55–23:14:04, `ci/web-soak.sh 3600 10000`, debug
build, 61 minta. A 3 kliens **minden percben ugyanannyi** eseményt kapott
(120), a socketek száma 3–4 (3 SSE + a mintavevő rövid kapcsolata), a
katalógus 10 000–10 001 (a szándékos add/remove churn).

A kapu: **RSS < 150 MB** és nincs SSE-szivárgás (a kliensek száma nem nő) —
mindkettő teljesül, ~16× margóval. A mérés a debug buildre vonatkozik;
release-ben az RSS jellemzően kisebb.

## Korlátok

- A daemon `MockEngine`: a valódi libtorrent Watch-forgalma sűrűbb lehet, de
  a coalesce ablak ezt is 1 üzenet/s-re fogja.
- A fizikai telefonos végigteszt (list → detail → pause → add → label) a user
  oldalán történik; a Playwright 390×844-es futás ugyanezt a DOM-ot méri.
- A LAN-on kívüli elérés (reverse proxy vagy port forward) még nyitott döntés.
