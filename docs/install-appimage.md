# Telepítés AppImage-ből (x86_64 Linux)

A `Ferrent-<verzió>-x86_64.AppImage` a desktop GUI önálló csomagja: a GTK-t
és a szükséges X/Wayland könyvtárakat tartalmazza, ezért bármely x86_64
Linuxon fut, ahol glibc ≥ 2.36 van (Debian 12+, Ubuntu 24.04+, Fedora 40+).
A daemon nincs benne — az AppImage a `ferrentd`-hez csatlakozik TLS-en.

Az rc1 fájl (`Ferrent-1.0.0-rc1-x86_64.AppImage`, **43 MB**) a GitLab
Generic Registryben van:

<https://cirice-gitlab.ddns.net/gabor/ferrent/-/packages> →
`ferrent-gui` → `1.0.0-rc1`.

## Futtatás

```bash
chmod +x Ferrent-1.0.0-rc1-x86_64.AppImage
./Ferrent-1.0.0-rc1-x86_64.AppImage
```

Első indításkor a **Servers** modal nyílik: add meg a daemon URL-jét
(`https://localhost:50051` vagy a távoli gép címe), a CA-t
(`tls-cert.pem`) és a tokent. A fingerprint-dialóguson hagyd jóvá a
tanúsítványt; utána a profil a `~/.config/ferrent/gui.toml`-ba kerül.

A tanúsítvány SAN-je `localhost` és `ferrent.local`; távoli daemonhoz a
`https://ferrent.local:50051` címet (vagy hosts bejegyzést) használd.

## FUSE nélkül

Ha a gépen nincs FUSE (konténer, régebbi kernel), az AppImage
kicsomagolás nélküli módja:

```bash
./Ferrent-1.0.0-rc1-x86_64.AppImage --appimage-extract-and-run
```

Ez ugyanazt a binárist futtatja egy ideiglenes könyvtárból; a profil és a
beállítások változatlanul a `~/.config/ferrent/` alatt maradnak.

## Ellenőrzés

A CI a `.deb`-ből telepített daemonhoz csatlakoztatja az AppImage-et
Xvfb alatt (`--appimage-extract-and-run --measure`), és a `rows=` sor
megjelenése a bizonyíték a TLS-profil helyességére (üres daemonnál
`rows=0`, ami szintén csatlakozást jelent). Részletek:
`packaging/appimage/smoke.sh`, mérés: `docs/measurements/packaging.md`.

## Eltávolítás

Az AppImage egy fájl; töröld, és ha kell, a `~/.config/ferrent/gui.toml`-t.
