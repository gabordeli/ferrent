# Telepítés macOS alá

**Állapot: tervezett.** macOS build még nem készült és nincs tesztelve; az
alábbi lépések a várható utat írják le, nem garantáltan működő receptet.

## Ami kell hozzá

- Xcode Command Line Tools (`xcode-select --install`).
- Rust stable (`rustup`), a `rust-toolchain.toml` szerint.
- libtorrent-rasterbar 2.0 és boost headers: `brew install libtorrent-rasterbar boost`.
- A GUI (iced) macOS-en a `winit`/Metal backendet használja; külön
  rendszerkönyvtár nem kell.

## Várható build

```bash
brew install libtorrent-rasterbar boost
cargo build --release -p ferrent-daemon --features libtorrent
cargo build --release -p ferrent-cli -p ferrent-gui
```

## Nyitott pontok

- A Ferrent motor `libtorrent` 2.0-ra épül; a Homebrew verziója követi
  az upstreamet, ezért a `Cargo.toml`/`docs/adr/0001-libtorrent-version.md`
  szerinti 2.0 sorozatot kell pinelni.
- A TLS önaláírt cert és a fingerprint pinning platformfüggetlen, de a
  tanúsítvány SAN-je (`localhost`, `ferrent.local`) macOS-en is a
  `ferrent.local` nevet igényli a távoli eléréshez.
- A daemon systemd helyett `launchd` plist-t kapna; ez még nincs megírva.
- Az AppImage nem macOS-formátum; a GUI `.app` bundle-t külön kell
  csomagolni.

Ha elkészül, ez a doksi kapja a működő lépéseket; addig a Linux az
ajánlott platform ([docs/install-debian.md](install-debian.md)).
