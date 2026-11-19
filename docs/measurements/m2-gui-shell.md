# M2-T2 – GUI shell, kapcsolat, TLS pinning

Ellenőrizve 2026-11-19, Debian 13.7, XWayland (`WINIT_UNIX_BACKEND=x11`),
1100×700.

## Mit tud

- Szerverprofilok: név, URL, CA PEM, API token; mentés
  `~/.config/ferrent/gui.toml` alá (vagy `$XDG_CONFIG_HOME`).
- A token `0600` módú configban van, a beviteli mező maszkolt; a token nem
  kerül logba.
- Első csatlakozáskor a CA certificate SHA-256 ujjlenyomatát mutatja, és
  csak a „Trust and connect” után tárolja (TOFU pin).
- Pinelt egyezésnél azonnal csatlakozik; eltérésnél külön figyelmeztetést
  mutat a várt és az aktuális ujjlenyomattal.
- Sikeres csatlakozás után 1 s-os poll a torrent listára; hibánál eldobja a
  klienst, `reconnecting` állapotba vált, és 1 s múlva újrapróbál.
- A státuszsáv minden állapotot és hibát a UI-ban mutat; nincs panic.
- Daemon-restart után a GUI újraindítás nélkül újracsatlakozik.

## Képernyőképek

| Képernyő | Fájl |
|---|---|
| Profil + kapcsolat | `docs/measurements/m2-gui-connect.png` |
| Első pin megerősítése | `docs/measurements/m2-gui-fingerprint.png` |
| Pin-elt eltérés figyelmeztetés | `docs/measurements/m2-gui-mismatch.png` |
| Csatlakozott főnézet | `docs/measurements/m2-gui-main.png` |

A mismatch nézet a várt (`Pinned fingerprint`) és a most bemutatott
(`Presented SHA-256 fingerprint`) értéket is mutatja. A „Trust and connect”
után a config az új ujjlenyomatot tárolja; a GUI újraindítva már dialógus
nélkül, a tárolt pinnel csatlakozik.

A főnézet a mock daemon `gui-demo` torrentjét mutatja `connected` állapotban.
A pin a kézi ellenőrzés után a configban:
`5f:34:df:89:7f:ef:80:b2:e9:26:73:f5:3e:27:a8:42:01:a0:da:ce:f7:54:a2:6d:67:c1:3b:5f:51:a5:f1:9c`.

## Automatikus tesztek

```bash
cargo test -p ferrent-gui
```

- `config`: profil roundtrip, `0600` mód, first/match/mismatch pin döntések.
- `protocol::tls`: ismert SHA-256 formátum.
- `tests/connection.rs::reconnects_after_daemon_restart`: in-process daemon
  (`MockEngine`, `MemoryStore`, ugyanaz a port és certificate); csatlakozás és
  lista, kliens eldobása, daemon leállítása, sikertelen újracsatlakozás
  ellenőrzése, majd ugyanazon a porton újraindított daemonhoz való
  újracsatlakozás és lista.

## Korlátok

- A token jelenleg plaintext a `0600` configban (a roadmap szerint
  elfogadható); keyring integráció későbbi feladat.
- A főnézet egyelőre az első 100 torrentet kéri le; a virtualizált,
  szerveroldalon szűrt tábla az M2-T3 feladata.
