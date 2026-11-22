# Deluge → Ferrent migráció

A Deluge állapotkönyvtárából a torrentek és a címkék átemelhetők; a
pluginok konfigurációja beolvasva, de nem alkalmazva. A import ugyanazt a
kódot használja a GUI-ban (bal oldali sáv → Importálás) és a CLI-ben
(`ferrent-cli import-deluge`), így az előnézet és a végrehajtás nem térhet el.

## Hol van a forrás

A daemon hostján:

| Deluge | Példa |
|---|---|
| state (`.torrent` fájlok) | `/var/lib/deluge/.config/deluge/state` |
| plugin configok | ugyanaz a könyvtár, vagy a `state/` szülője |

A Deluge a torrenteket az info-hash-ről elnevezett fájlokban tartja; a
`label.conf`, `autoadd.conf`, `execute.conf` és `scheduler.conf` JSON.

## Mit visz át

- **Torrentek**: minden `.torrent` a state könyvtárból (rekurzív scan nélkül,
  egy szinten).
- **Címkék**: a `label.conf` deklarált címkéi (név + szín); a hiányzók
  létrejönnek az import előtt. A torrent–címke hozzárendelés az info-hash
  alapján megy át.
- **Dedup**: a daemonban már létező info-hash kimarad (`skipped`), nem hiba.
- **Állapot**: a torrentek **paused** kerülnek be, hogy az import után
  ellenőrizhetők legyenek; a folytatás külön művelet.

## Mit nem visz át

- **Resume/fastresume** adat: az első indulásnál a daemon ellenőrzi a
  fájlokat (recheck nélkül, a resume data hiányában a libtorrent a
  fájlméretekből dolgozik).
- **Ratio, összes feltöltés/letöltés, queue-sorrend**: a Deluge számlálói nem
  jönnek át.
- **Pluginok működése**: az `autoadd.conf`/`execute.conf`/`scheduler.conf`
  tartalma az előnézetben figyelmeztetésként/mezőként látszik, de a Ferrent
  saját `[autoadd]`/`[[hook]]`/`[scheduler]` konfigurációját nem írja át.
- **Session-beállítások** (sebességlimitek, port, proxy): a Ferrent
  `config.toml`-ja marad az igazság.

## Lépések

1. Állítsd le a Deluge-ot, vagy legalább ne írjon a state könyvtárba.
2. GUI: **Importálás** (bal oldali sáv) → állapotkönyvtár → **Előnézet**.
   CLI: `ferrent-cli import-deluge --state-dir <dir> --dry-run`.
3. Szűrj (név/tracker regex, mentési út, címke), és ellenőrizd a listát
   (darabszám, már megvan, címkék, figyelmeztetések).
4. Jelöld ki az importálandókat (vagy **Összes találat kijelölése**),
   majd GUI: **Importálás szüneteltetve**; CLI: `--dry-run` nélkül.
5. A torrentek `paused` állapotban jelennek meg; a szükségeseket indítsd
   (resume), vagy állítsd be a címkék zónáit/limiteit előbb.

## Ismert korlát

- A privát trackeres torrentek import után announce-olhatnak, ha resume-olod
  őket; a Deluge privát tracker konfigurációja nem megy át.
- A state könyvtár a **daemon** hostján értelmezett útvonal; a GUI a saját
  gépén nem látja.
