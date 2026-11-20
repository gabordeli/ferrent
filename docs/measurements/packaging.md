# Csomagolás (M3-T1) – mérések

Mérve 2026-11-20, Debian 13 host, 12 szál (i7-1355U), docker 29.9.1,
`cargo-deb` 3.8.0, `cargo` 1.100.0.

## M1 – A csomag

| Build | Parancs | Idő | `.deb` | Telepített méret |
|---|---|---:|---:|---:|
| Debian 13 (host) | `cargo deb -p ferrent-daemon --features libtorrent` | 22,5 s (meleg target) | 3,79 MB | 14,9 MB |
| Debian 12 (`debian:12`, tiszta toolchain) | ugyanaz konténerben | 2 m 12 s (teljes release) | 3,79 MB | 14,9 MB |

Tartalom: `/usr/bin/ferrentd`, `lib/systemd/system/ferrentd.service`,
`/etc/ferrent/config.toml` (conffile), `man1/ferrentd.1.gz`, copyright.
A token **nincs** a csomagban: a `postinst` generálja 32 bájtos base64-ként
(44 karakter) `/etc/ferrent/token`-be, `0640 root:ferrent`.

**A függőségek a build host suite-jét követik** (`$auto`):

| Suite | Depends |
|---|---|
| bookworm | `libc6 (>= 2.34), libssl3, libtorrent-rasterbar2.0 (>= 2.0.8)` |
| trixie | `libc6 (>= 2.39), libssl3t64, libtorrent-rasterbar2.0t64 (>= 2.0.11)` |

Egy trixie-n épült `.deb` tehát **nem telepíthető** bookwormra (libc6 2.36 < 2.39,
nincs `libssl3t64`/`t64` libtorrent). A cél-suite build konténer az M3-T2 dolga;
addig a `packaging/debian/README.md` recipe-je szerint kell építeni.

## M2 – Telepítés friss konténerbe

`packaging/debian/container-test.sh` — 31 assert, Debian 12 **és** Debian 13
ellen: **31/31 PASS**, exit 0 mindkettőn.

| Ellenőrzés | Eredmény |
|---|---|
| `ferrent` system user/group, shell `nologin` | PASS |
| `/var/lib/ferrent` `0750 ferrent:ferrent` | PASS |
| `config.toml` / `token` `0640 root:ferrent` | PASS |
| token = 32 bájt → 44 karakter | PASS |
| `postinst` enableli a unitot (systemd fut) | PASS |
| `systemctl start` → active, `127.0.0.1:50051` listen | PASS |
| `startup_ms` 8,8–16,1; RSS ~9,5 MB (üres állapot) | PASS |
| `systemctl stop` 33 ms, `saved fastresume` | PASS |
| restart új PID, TLS cert + fingerprint változatlan | PASS |
| `apt remove`: state/config/token megmarad, binary/unit eltűnik, service leáll | PASS |
| `apt purge`: `/var/lib/ferrent`, `/etc/ferrent`, token törölve | PASS |
| `apt purge`: a `ferrent` user **megmarad** (dokumentált) | PASS |

## M3 – Konténer-specifikus csapdák (mind a szkriptben kezelve)

- A `debian:*-slim` képek `/usr/sbin/policy-rc.d`-je `101`-gyel tér vissza, így
  a `prerm` `deb-systemd-invoke stop`-ja csendben nem állítja le a szervizt
  (a `remove` után „active" marad, a binary már törölve). A teszt törli a
  `policy-rc.d`-t; valódi Debianon nincs ilyen.
- A kép build közben nincs futó systemd, ezért a `postinst`
  `deb-systemd-helper enable` ága kimarad. A teszt `--reinstall`-lal futtatja
  újra a `postinst`-ot a futó systemd alatt, és azt is ellenőrzi.
- systemd PID1-ként csak `--cgroupns=host` + `-v /sys/fs/cgroup:/sys/fs/cgroup:rw`
  mellett indul; `:ro` esetén a konténer azonnal `255`-tel kilép, üres loggal.
- A unit útja `systemctl show -p FragmentPath` szerint
  `/lib/systemd/system/...` (merged-usr); a teszt nem hardcode-ol `/usr/lib`-ot.

## M4 – Ismételhető ellenőrzés

```bash
packaging/debian/container-test.sh debian:12 target/debian/ferrentd_0.1.0-1_amd64.deb
packaging/debian/container-test.sh debian:13 target/debian/ferrentd_0.1.0-1_amd64.deb
```

A szkript a `.deb`-ből épít egy eldobható systemd-s képet, lefuttatja a 31
assertet, és `exit 1`-gyel áll meg az első bukásnál (a PASS/FAIL sorok
megmondják, melyik).
