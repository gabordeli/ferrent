# Ferrent web – telepítés LAN-on (HTTPS, reverse proxy nélkül)

A `ferrent-web` maga szolgálja ki a SPA-t **és** terminálja a TLS-t, így a
belső hálón nincs szükség nginxre. A daemon továbbra is loopbacken figyel; a
LAN felé csak a gateway néz.

## 1. Build

```bash
cargo build --release -p ferrent-web
(cd webui && npm ci && npm run build)      # webui/dist
```

A `ferrent-web` a `spa_dir`-ből szolgálja ki a SPA-t (fallback: `index.html`).

## 2. Konfiguráció

`/etc/ferrent/web.toml` (minta: `packaging/debian/web.toml`):

```toml
listen = "0.0.0.0:8443"
password = "…"                 # a belépési jelszó
read_only = false              # true = csak olvasás (pause/add/label letiltva)
spa_dir = "/usr/share/ferrent-web"

[tls]
cert = "/var/lib/ferrent/web-cert.pem"
key  = "/var/lib/ferrent/web-key.pem"
names = ["ferrent.local", "192.168.2.65"]   # amit a telefon megnyit

[daemon]
url  = "https://localhost:50129"
ca   = "/var/lib/ferrent/tls-cert.pem"
token = "…"                    # a daemon tokenje (0640)
```

- A **certificate first startkor generálódik** (SAN: `names` + `localhost` +
  `127.0.0.1`/`::1`), a kulcs 0600. A startup log kiírja a **SHA-256
  fingerprintet**; a telefonon a tanúsítvány-elfogadásnál ez ellenőrizhető.
- A jelszó a configban él; a session cookie `HttpOnly; SameSite=Strict`, HTTPS
  alatt `Secure`.
- A daemon **portja ne legyen publikus**, és a `url` **`localhost`** legyen:
  a daemon tanúsítványának SAN-je `localhost` + `ferrent.local`, IP-literal
  (`https://127.0.0.1:…`) ellen a TLS-ellenőrzés elbukik, a gateway pedig
  `daemon connect failed: transport error`-t ír.
- `read_only = true` esetén a pause/add/label mind `403`; ez a „csak nézem”
  telefonnak vagy egy falikijelzőnek való.

## 3. Indítás

```bash
install -m 644 packaging/systemd/ferrent-web.service /lib/systemd/system/
systemctl daemon-reload
systemctl enable --now ferrent-web
journalctl -u ferrent-web -n 20
```

A unit `User=ferrent`, `ProtectSystem=strict`, `ReadWritePaths=/var/lib/ferrent`
(mert a cert/key oda íródik).

## 4. Első használat a telefonon

1. `https://<host>:8443` – a böngésző egyszer megkérdezi a self-signed certet;
   a fingerprint a startup logban van.
2. Belépés a jelszóval (a „Felhasználónév” mező előtöltött, a gateway nem
   használja).
3. Lista → detail → pause/resume → add (magnet vagy `.torrent`) → label.

A LAN-on kívüli (publikus) elérés **későbbi döntés**: akkor vagy egy meglévő
443-as reverse proxy alá kerül a gateway (path vagy külön port), vagy port
forward + a saját cert cseréje publikus névre. Addig a 8443 LAN-port elég.

## 5. Tűzfal

A gateway portját a LAN felé kell engedni (a daemonét nem):

```bash
ufw allow from 192.168.2.0/24 to any port 8443 proto tcp
```

## 6. Frissítés

A `webui/dist` cseréje után elég a `ferrent-web` újraindítása; a daemon és a
natív kliensek változatlanok. A cert/key megmarad, a böngésző sessionje is
(ha a `session_ttl_secs` még él).

## 7. Terhelés-ellenőrzés (M7-T4)

A gateway a 10 000 torrentes katalógust és a három SSE klienst egy órán át
bírja-e, a `ci/web-soak.sh` méri — maga indít egy demó daemont, három SSE
klienst és 30 s-onként egy add/remove-ot, majd percenként mintát vesz
(RSS, nyitott SSE socketek, kliensenkénti eseményszám):

```bash
cargo build --release -p ferrent-web -p ferrent-gui --example demo_server
ci/web-soak.sh 3600 10000
# → /tmp/ferrent-web-soak/gateway.csv
```

A kapu: RSS < 150 MB és nincs SSE-szivárgás (a socketek száma nem nő). A
mért értékek a `docs/measurements/webui.md` M4 szakaszában vannak.
