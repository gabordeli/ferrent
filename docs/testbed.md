# Teszt-háló (LAN) – szerepek és beüzemelés

Cél: a `ferrentd` valódi peer-forgalommal, publikus trackerek nélkül
igazolható legyen, és a kiadási kapu (**≥6 hónap éles, stabil futás**)
teljesüljön.

## Szerepek (mérve 2026-11-20)

| Gép | Szerep | Arch | Megjegyzés |
|---|---|---|---|
| **tracker** `192.168.2.116` | saját C tracker (`ferrent-tracker`) + katalógus-UI | armv7l (Orange Pi, Debian 9) | 238 MB RAM, nincs Python; a role csak `raw` modulokat használ |
| **seed** `192.168.2.28` (caprice) | `ferrentd` 0.1.2 (fő daemon 6881) + `ferrent-load` (10 000 torrent, 6882) | amd64 (M910q, 4 mag/32 GB) | ufw: `6881:6891` TCP+UDP a `192.168.2.0/24`-ről |
| **kliens (terhelés)** `192.168.2.26` (cerebellum) | `ferrentd` + 1 000 `.torrent` AutoAdd-dal | amd64 | 8 GB szabad, ezért csak ~1 GB payload |
| **laptop** | `ferrentd` + GUI + `ferrent-web` | amd64 | a `.27`-en közben Deluge marad; a GUI innen validál |

A telepítés **csak** Debian 13 + SSH; minden más playbookból megy, az
`~/docker-images/infra` repóban (`deploy_ferrent_tracker.yml`,
`deploy_ferrent_seed.yml`).

## Sorrend

1. **Verziózás + release pipeline**: tag → `.deb` verzió, GitLab release job,
   generic registry (`ferrentd 0.1.2-1`).
2. **Gépek**: Debian + SSH, fix DHCP-rezerváció, hostnevek.
3. **Tracker a Pi-n**: `deploy_ferrent_tracker.yml` → `/announce`, `/scrape`,
   `/api/torrents`, `/upload`, `/t/<hash>.torrent`, `/health`, a UI a 6969-en.
4. **Seed a caprice-on**: `deploy_ferrent_seed.yml` → `.deb` a registryből,
   `mktorrent` payloadok, `ferrent-load` a 10 000 torrenttel.
5. **Kliens**: a laptop `ferrentd` + GUI, illetve a cerebellum a terheléshez,
   a tracker `http://192.168.2.116:6969/announce` címével.
6. **Forgatókönyvek** (rögzítve a `docs/measurements/` alá):
   - letöltés → befejezés → seeding, valódi seeds/peers szám;
   - restart recheck nélkül mindkét oldalon;
   - label move-on-complete + DiskGuard staging → library rename;
   - AutoAdd watch dir-ből;
   - 10 000 torrentes soak (lásd `docs/measurements/mvp.md`);
   - később: ratio auto-remove (M8-T3), VPN-kötés (M8-T4).

## Mért tracker-viselkedés

- **Kapacitás:** 10 003 torrent / 12 400 peer mellett **~10 MB RSS, ~3% CPU**
  (a 238 MB-os Pi-n); a katalógus betöltése ~26 s, a bind csak utána jön.
- **Peer-tábla perzisztálás:** `<state_dir>/peers.db`, 30 s-onként atomikusan
  írva, `SIGTERM`-re azonnal flush-olva; restart után a peer-ek visszatöltenek
  (mért: 1 540 peer a restart után 20 s-mal) — a kliensek nem esnek ki.
- **UI:** keresés, mind a hat oszlopra rendezés, **lapozás** (100/200/500/1000
  sor), `.torrent` feltöltés/letöltés (a mentett fájlnév a torrent neve),
  magnet másolás, törlés. 10 000 sornál a lapozott render ~10 ms
  (a korábbi, teljes újraépítés 498 ms volt).

## Nyitott kérdés

A Pi arch-ja csak akkor számít, ha később `ferrentd` futna rajta (akkor kell
`armhf` vagy `arm64` build). Trackernél nem.
