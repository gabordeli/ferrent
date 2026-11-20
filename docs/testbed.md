# Teszt-háló (LAN) – beüzemelési sorrend

Cél: a `ferrentd` valódi peer-forgalommal, publikus trackerek nélkül
igazolható legyen, és a kiadási kapu (≥5 torrent, ≥7 nap) teljesüljön.

## Szerepek

| Gép | Szerep | Arch | Megjegyzés |
|---|---|---|---|
| Orange Pi | tracker + minimális web UI | bármilyen (a tracker nem a mi csomagunk) | 256 MB RAM elég; a UI csak státuszt mutat |
| régi laptop (1 GB) | seed server | valószínűleg amd64 | a 72 h soak ide kerül, nem a munkaállomásra |
| a laptopod | `ferrentd` + GUI + `ferrent-web` | amd64 | a .27-en közben Deluge marad |
| .27 | Deluge (változatlan) | amd64 | később `import-deluge` valós forrása |

A telepítés **csak** Debian 13 + SSH; minden más playbookból megy, az
`~/docker-images/infra` repóban (ott vannak a szerverek playbookjai).

## Sorrend

1. **Verziózás + release pipeline** (ez a commit): a tag → `.deb` verzió, GitLab
   release job, Debian Package Registry.
2. **Gépek**: Debian 13, SSH (user + sudo), fix DHCP-rezerváció, hostnevek.
3. **Tracker a Pi-n**: opentracker vagy egy kis Node-os tracker konténerben,
   `/announce` + egy státusz HTML; port 6969 TCP/UDP a LAN-on.
4. **Seed a régi laptopon**: `ferrentd` a registryből (`apt install ferrentd`),
   token, egy teszt-torrent elkészítése (`mktorrent`), seed indítása.
5. **Kliens a laptopon**: a meglévő `ferrentd` + GUI, a tracker
   `http://<pi>:6969/announce` címével, a seed-től letöltés.
6. **Forgatókönyvek** (mindegyik rögzítve a `docs/measurements/` alá):
   - letöltés → befejezés → seeding, valódi seeds/peers szám;
   - restart recheck nélkül mindkét oldalon;
   - label move-on-complete + DiskGuard staging → library rename;
   - AutoAdd watch dir-ből;
   - 72 h soak a seed laptopon (RSS, fd, crash);
   - később: ratio auto-remove (M8-T3), VPN-kötés (M8-T4).

## Amit a Pi-n érdemes mérni

- tracker RAM/CPU (elvárás: < 20 MB, ~0% CPU);
- announce-latencia a LAN-on (< 5 ms);
- a tracker újraindítható-e a peers megszakítása nélkül (a kliensek
  újra-announce-olnak).

## Nyitott kérdés

A Pi arch-ja csak akkor számít, ha később `ferrentd` futna rajta (akkor kell
`armhf` vagy `arm64` build). Trackernél nem.
