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
url  = "https://127.0.0.1:50129"
ca   = "/var/lib/ferrent/tls-cert.pem"
token = "…"                    # a daemon tokenje (0640)
```

- A **certificate first startkor generálódik** (SAN: `names` + `localhost` +
  `127.0.0.1`/`::1`), a kulcs 0600. A startup log kiírja a **SHA-256
  fingerprintet**; a telefonon a tanúsítvány-elfogadásnál ez ellenőrizhető.
- A jelszó a configban él; a session cookie `HttpOnly; SameSite=Strict`, HTTPS
  alatt `Secure`.
- A daemon **portja ne legyen publikus**: `url` marad `127.0.0.1`.

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
