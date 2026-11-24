# iOS kliens – kutatás (M13-T12)

> **Állapot (2026-11-24):** a PWA-telepítés nem elosztási út (10-ből 2 tudja a
> kezdőképernyőre tenni) → a döntés Capacitor shell (#84). A #84
> felülvizsgálat a **natív RPC** utat ajánlja, ha a cél a közvetlen
> `ferrentd`-kapcsolat és az App Store; az iOS-munka egyelőre **parkol**. Az
> alábbi „Javasolt út" a 2026-11-23-i PWA-első állapotot írja le; a
> 2026-11-24-i szakasz a végén van.

Kérdés: milyen úton kap a Ferrent iOS klienst, és mennyi munka az első
használható verzió? A daemon a torrenteket futtatja; az iOS kliens **csak
menedzsel** (lista, pause/resume, hozzáadás, részletek) — háttér-seeding nem
cél, mert azt a szerver végzi.

## Opciók

| | **A. PWA** (a meglévő SPA) | **B. Capacitor / WKWebView** | **C. Natív SwiftUI + grpc-swift** |
|---|---|---|---|
| UI | a `webui/` SPA (mobil-first, 390×844) | ugyanaz a SPA natív shellben | újraírt SwiftUI |
| API | REST + SSE a `ferrent-web` gatewayen | REST + SSE | közvetlen gRPC (TLS) vagy REST |
| TLS | böngésző trust: a gateway certje (self-signed) vagy egy valódi cert | ATS kivétel + WKWebView delegate | `NIOSSL` trust root + fingerprint pinning |
| Háttér | nincs (iOS felfüggeszti) | nincs | korlátozott; a munkát a daemon végzi |
| Push | Web Push (iOS 16.4+, **csak** home-screen PWA) — a gatewayben ma **nincs** VAPID | APNs egy kis backenddel | APNs |
| App Store | nem kell (home screen) | App Store-ra kész (review kell) | App Store |
| Megosztás (`magnet:` / `.torrent`) | `share_target` a manifestben (Safari share sheet → `/share`) | ugyanaz + natív share extension | natív share extension |
| Munka (első verzió) | **1–2 nap** (safe-area, install hint, teszt) | **3–5 nap** + review | **3–4 hét** |
| Kockázat | a self-signed cert trust, push hiánya | review + a WebView „nem elég natív” érzés | TLS pinning + a teljes UI újraírása |

## Mért tények a repóból

- A SPA **már PWA**: `webui/public/manifest.webmanifest` (`display:
  standalone`, `orientation: portrait`, 192/512/apple-touch ikonok,
  `share_target` a `/share` végpontra), `sw.js` (cache-first shell,
  hálózat-first API, `push`/`notificationclick` handlerek), `push.ts`
  (`PushManager.subscribe` VAPID kulccsal).
- A REST felület a mobil v1-hez elég: `GET /api/torrents` (szűrés,
  lapozás), `/api/torrents/{id}` + `/files`/`/peers`/`/trackers`/`/webseeds`,
  `POST /api/torrents` (magnet vagy base64 metainfo), pause/resume/remove,
  bulk, `GET /api/labels`, `GET|PUT /api/config`, SSE `/api/events`
  (`docs/web-api.md`).
- **A push szerveroldala hiányzik**: a gateway `crates/web` nem tartalmaz
  VAPID/push végpontot, a `PushToggle` kliens-oldali stub. Web Push-hez a
  gatewayben kell: subscriptions tároló, VAPID kulcspár, titkosított küldés
  (`aes128gcm`), és a daemon értesítéseinek becsatolása.
- **TLS**: a gateway a saját, first startkor generált certjével terminál
  (`web.toml` `[tls]`), a daemon loopbacken marad. iOS-en a self-signed
  certet vagy **profilként** kell telepíteni (Settings → General → About →
  Certificate Trust Settings), vagy a gateway kaphat valódi certet (pl.
  Let's Encrypt DNS-01 a `ferrent.<domain>` névre) — utóbbi a tiszta út.
- **Helyi hálózat**: ha az app a LAN-on a gatewayet nyitja, iOS 14+ alatt
  `NSLocalNetworkUsageDescription` + a felhasználó engedélye kell; távoli
  elérésnél (a gateway publikus HTTPS-e) ez nem téma.

## Javasolt út

**Az első verzió a PWA** (A), mert a munka 90%-a kész:

1. **Telepíthetőség**: „Hozzáadás a főképernyőhöz” hint, safe-area
   (iPhone notch), `apple-mobile-web-app-*` meták, landscape tiltás ellenőrzése.
2. **Valódi cert a gatewayre** (Let's Encrypt DNS-01) — ezzel a Safari
   trust-gond és a profil-telepítés megszűnik; a daemon certje marad
   pinningre a gateway felé.
3. **Web Push** a gatewayben (VAPID + subscription store + `aes128gcm`),
   a daemon értesítéseiből — ez adja a „befejeződött” jelzést zárt appnál is.
4. **App Store megjelenés** csak akkor, ha kell: a PWA-t **Capacitor**-ral
   be lehet csomagolni (B), egy kódbázissal; a review-hoz review-notes a
   self-hosted szerverről.

**Natív (C)** akkor, ha a háttér/push/natív érzés vagy a nagy torrentszámú
lista teljesítménye ezt megköveteli; a TLS pinning `NIOSSL`-lal megoldható,
de a teljes UI újraírás 3–4 hét.

## Döntési pontok (user)

- **PWA elég, vagy App Store-os app kell?** Ha store: Capacitor (B) a
  leggyorsabb, natív (C) a legdrágább.
- **Kell-e push az első verzióban?** Ha igen, a gateway VAPID munkája
  (~2–3 nap) megelőzi a klienst.
- **Távoli elérés**: a gateway publikus HTTPS-e (valódi cert) vagy VPN —
  utóbbi esetén a cert-kérdés a VPN-en belül marad.

## Becslés a v1.0.0-rc1 után

| Lépés | Munka |
|---|---|
| PWA polírozás (safe-area, hint, teszt) | 1–2 nap |
| Valódi cert a gatewayre (DNS-01) | 0,5 nap + DNS |
| Web Push a gatewayben | 2–3 nap |
| Capacitor wrapper + TestFlight | 2–3 nap + review |
| Natív SwiftUI v1 (lista, detail, add, pause) | 3–4 hét |

Külön repo nem kell a PWA-hoz (a `webui/` és a `ferrent-web` marad); a
Capacitor shell és a natív app külön repóban vagy a `mobile/` alatt élhet.

## 2026-11-24-i felülvizsgálat (#84)

**Új feltételek (user):** App Store-ba feltölthető, értékelhető app;
**100%-ig kompatibilis a `ferrentd` RPC-protokolljával** (nem REST a
gatewayen); nincs macOS (Linux asztal, iPhone 7 + iPhone 14, az iOS 15 a
plafon); a developer fiókot a user intézi. A PWA-telepítés nem elosztási út
(10-ből 2).

### Mért tények

- A proto **44 RPC-t** és egy szerver-streamet (`Watch`) ad
  (`crates/protocol/proto/ferrent/v1/ferrent.proto`).
- A daemonban **nincs gRPC-Web** (`tonic-web` sehol); a `webui/` SPA **REST-et**
  hív a `ferrent-web` gatewayen (`docs/web-api.md`), relatív `/api` útvonalakkal,
  és a gatewayen **nincs CORS**.
- A daemon RPC-je **TLS + token + self-signed cert**; a desktop GUI
  fingerprint-pinningel (TOFU).
- **Web Push** iOS 16.4+ és home-screen PWA → az iPhone 7 (iOS 15) nem tudja;
  a push tehát **APNs**.
- **Minden iOS build macOS-t kér** (Capacitor is); a self-hosted GitLabon nincs
  macOS runner → GitHub `macos-latest`, Codemagic vagy Xcode Cloud.
- **App Store review:** a reviewer nem éri el a user daemonját → **demo mód**
  (`crates/gui/examples/demo_server.rs` mock engine) vagy publikus, read-only
  demo daemon kell.

### Utak

| Út | RPC | TLS pinning | Review | Munka |
|---|---|---|---|---|
| **Natív SwiftUI + grpc-swift** | igazi gRPC/HTTP2, proto-ból generálva | igen (NIOSSL) | natív UI, kisebb 4.2 kockázat | 2–3 hét v1 |
| **Capacitor + natív gRPC híd** | a Swift híd beszél gRPC-t | igen (a hídban) | webview-wrapper kockázat | ~1–1,5 hét |
| **Capacitor + gRPC-Web** | `tonic-web` + CORS a daemonban, a SPA transportjának átírása | nem (WebView) | webview | 1–2 hét + SPA refactor |

Az **Expo + React Native** (#84) egy harmadik UI, és a gRPC-hez saját natív
modul/dev build kell — a Mac nélküli előny csak a UI-fejlesztésre igaz.

**Javaslat:** ha a „100% RPC" **közvetlen daemon-kapcsolatot** jelent, a
**natív SwiftUI + grpc-swift** a tiszta út; a Capacitor + natív híd a második,
ha a web UI-t meg akarjuk tartani. A PWA marad a desktop/Android böngészőkre.

### Szükséges hozzá (checklist)

- [ ] **Apple Developer Program** (99 USD/év), App Store Connect, signing
      (cert + provisioning), TestFlight (internal, majd external).
- [ ] **macOS build út**: GitHub `macos-latest` a tükörön (Actions engedélyezve,
      macOS-minuták) **vagy** bérelt Mac / használt Mac mini; rögzített Xcode.
- [ ] **App Store**: demo mód vagy publikus demo daemon, privacy manifest,
      encryption declaration, screenshotok, leírás, support URL, age rating;
      a torrent-vezérlő app review-kockázata külön.
- [ ] **APNs**: `.p8` kulcs + Team ID + Key ID; gateway/daemon oldali küldő
      (ES256 JWT + HTTP/2) + device-token store. Ez a **#82 iOS-felét váltja**;
      a VAPID-os Web Push legfeljebb desktopra marad.
- [ ] **iOS 15 target**: `NavigationView` (nem `NavigationStack`), grpc-swift
      minimum verzió ellenőrzése, iPhone 7-es teszt.
- [ ] **Hálózat**: a daemon gRPC-portja elérhető (portforward/VPN); LAN-on
      `NSLocalNetworkUsageDescription`; cert profilként **vagy** TOFU pinning
      (a natív app tudja).
- [ ] **Prototype (2–3 nap)**: proto→Swift generálás, connect + `ListTorrents`
      + `Watch` + pause/resume a valódi daemon ellen, CI-ból.

**Állapot (2026-11-24): parkol** (user: „nagyjuk egyelőre az ios-t"); a
következő lépés a prototype, amint van macOS build út.
