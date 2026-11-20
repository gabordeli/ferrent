# ADR 0010 – A web gateway külön binary

**Státusz:** elfogadva (2026-11-20)
**Kontextus:** M7 – `ferrent-web` a böngészős felülethez. A daemon TLS gRPC-t
beszél bearer tokennel; a böngésző ezt közvetlenül nem tudja. A kérdés az
volt, hogy a web gateway a daemonba épüljön-e, vagy külön folyamat legyen.

## Döntés

Külön crate és binary: `ferrent-web` (`crates/web`), amely

- egyetlen TLS gRPC kapcsolatot tart a daemonhoz (a CA a trust anchor,
  opcionálisan explicit SHA-256 fingerprinttel),
- cookie-alapú sessionnel védett HTTP felületet ad (login/logout, `/health`,
  `/api/events` SSE),
- a daemon `Watch` streamjét **egy** előfizetésként olvassa, és 1 s-os
  ablakban coalescálva fan-outolja a klienseknek,
- TLS-t **nem** terminál: nginx (vagy más reverse proxy) végzi, a gateway
  loopbacken figyel.

## Indokok

1. **Üzemeltetés:** a web és a daemon külön indítható/újraindítható; a SPA
   cseréje nem igényel daemon-buildet, a daemon frissítése nem töri a webet.
2. **Felület:** a daemon egyetlen, tokennel védett gRPC surface marad; a web
   réteg (session, rate limit, JSON) nem kerül a daemonba, így a natív
   kliensek és a CLI változatlanok.
3. **Terhelés:** a 10 000 torrentes `Watch` olvasása és a JSON-renderelés a
   web folyamatban történik; a daemon CPU-ja nem nő a böngészők számával.
4. **Biztonság:** a daemon portja nem publikus; a web tokenje a web configban
   él, a böngésző csak egy HttpOnly session cookie-t lát. Read-only módban a
   gateway minden mutációt elutasít (T2).
5. **TLS:** a tanúsítványkezelést a meglévő nginx/edge proxy végzi (a
   `.228`-on már futó minta), így nincs duplikált cert-kezelés.

## Következmények

- Két unit/telepítési egység (`ferrentd`, `ferrent-web`), a web config külön
  fájlban (`/etc/ferrent/web.toml`), benne a daemon token.
- A gateway restartja minden böngésző-sessiont eldob (memória-session) – ez
  egyfelhasználós dashboardnál elfogadható.
- A session/rate-limit állapot nem perzisztens; több gateway példány mögött
  sticky session kellene (nem cél).
- Ha később mégis in-process TLS kell, a `main.rs`-ben egy `rustls` listener
  becsatolható anélkül, hogy a route-ok változnának.

## Elvetett alternatívák

- **Beépítés a daemonba (feature flag):** kisebb deploy, de a SPA/build és a
  session-kezelés a daemonba kerül, és minden webfix daemon-release.
- **Közvetlen gRPC-web a böngészőből:** a tokent a böngésző kapná meg, és a
  daemon portja publikus lenne – ez sérti a jelenlegi tokennel védett,
  portot rejtő modellt.
