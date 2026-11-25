# Ismert korlátok

Ezek a korlátok az 1.0.0-rc1 állapotra érvényesek; a mérés és a nyitott
hibák: [docs/measurements/v1-rc1.md](measurements/v1-rc1.md).

## Funkcióhiány

- Nincs kicsomagoló (extractor), többoldalas kereső, HTTP streaming és
  uTorrent-import; a Deluge-import viszont kész
  (`ferrent-cli import-deluge`, [docs/deluge-migration.md](deluge-migration.md)).
- Nincs torrent-név-átnevezés (a fájlnév-átnevezés kész), és nincs
  darabtérkép nézet (GUI/web/CLI).
- Nincs port-teszt, random port és UPnP mapping státusz; a config
  kapcsolói megvannak.
- Nincs SMTP e-mail értesítés (webhook, Telegram és Web Push van); v1-ben
  nem cél.
- A web UI a desktop GUI funkcionalitásának egy részét fedi le
  ([docs/web-api.md](web-api.md)): nincs Import oldal és torrentenkénti
  queue-move; a torrent options panel több jelölője még nem perzisztál.
- Nincs in-app auto-update (APT repó és NSIS telepítő van).

## Viselkedés

- A `.torrent` hozzáadás save pathja a **daemon hostján** értelmezett; a
  GUI nem ellenőrzi, hogy létezik-e.
- A GUI `STATE` oszlopa nem rendezhető szerveroldalon; a támogatott
  rendezési kulcsok: id, name, label, progress, download, upload, queue.
- A 24 órás soak a megadott mérési környezetre érvényes, nem párhuzamos
  daemonokra vagy több ezer aktív peerre.

## Biztonság

- A token plaintext a `0600` configban (keyring későbbi).
- A TLS tanúsítvány SAN-je `localhost` és `ferrent.local`; távoli gépről
  a `ferrent.local` név (vagy egy megfelelő hosts bejegyzés) szükséges.
  A tanúsítvány cseréjekor a kliensek fingerprint-eltérést jeleznek.
- A daemon a `.deb`-ben `127.0.0.1:50051`-en hallgat; távoli eléréshez
  a config `bind` értékét kell módosítani (és tűzfalat nyitni).

## Platformok

- A `.deb` Debian 12/13-ra és Ubuntu 24.04-re készül (amd64 + arm64; a GUI
  amd64). Fedora/Arch forrásból építhető
  ([docs/install-fedora.md](install-fedora.md),
  [docs/install-arch.md](install-arch.md)); a Windows **előnézet** (ZIP +
  NSIS, cross-build, futásban méretlen), a macOS-nek nincs buildje
  ([docs/platform-support.md](platform-support.md),
  [docs/install-windows.md](install-windows.md)).
- A Docker image fej nélküli daemon (nincs benne GUI és web gateway):
  [docs/install-docker.md](install-docker.md).

## Terhelés

- 4 000 torrent betölthető és a mirror a keretben marad
  ([mérés](measurements/mvp.md)); 10 000 torrentnél a startup és a lista
  hosszabb, és a daemon a teljes katalógust a memóriában tartja.
