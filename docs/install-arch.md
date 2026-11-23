# Telepítés Arch Linux alá

Arch-csomag (PKGBUILD) egyelőre nincs; a daemon, a CLI és a GUI forrásból
épül.

## 1. Függőségek

```bash
sudo pacman -S --needed base-devel pkgconf libtorrent-rasterbar boost openssl
```

A GUI-hoz (opcionális):

```bash
sudo pacman -S --needed libxkbcommon wayland libx11 libxcursor libxrandr \
  libxi glib2 gtk3
```

Rust: `sudo pacman -S rustup && rustup default stable`, vagy a
`rust-toolchain.toml` szerinti stable toolchain.

## 2. Build

```bash
git clone https://cirice-gitlab.ddns.net/gabor/ferrent.git
cd ferrent
cargo build --release -p ferrent-daemon --features libtorrent
cargo build --release -p ferrent-cli -p ferrent-gui
```

## 3. Futtatás

```bash
sudo install -Dm0755 target/release/ferrentd /usr/local/bin/ferrentd
sudo install -Dm0755 target/release/ferrent-cli /usr/local/bin/ferrent-cli
sudo install -Dm0755 target/release/ferrent-gui /usr/local/bin/ferrent-gui

sudo useradd --system --home /var/lib/ferrent --shell /usr/sbin/nologin ferrent
sudo install -d -o ferrent -g ferrent -m 0755 /var/lib/ferrent
sudo install -d -m 0755 /etc/ferrent

TOKEN="$(head -c 24 /dev/urandom | base64)"
echo "$TOKEN" | sudo tee /etc/ferrent/token >/dev/null
sudo chown root:ferrent /etc/ferrent/token && sudo chmod 0640 /etc/ferrent/token

sudo -u ferrent FERRENT_DATA_DIR=/var/lib/ferrent \
  FERRENT_BIND=127.0.0.1:50051 /usr/local/bin/ferrentd
```

A daemon a `tls-cert.pem`-et a `FERRENT_DATA_DIR` alá írja; a token a
`token_file` alapértelmezés (`/etc/ferrent/token`) szerint olvasódik.

## 4. systemd

Ugyanaz a unit, mint a [Fedora-doksi](install-fedora.md#4-systemd-opcionális)
4. szakaszában (`/etc/systemd/system/ferrentd.service`).

## Eltávolítás

```bash
sudo systemctl disable --now ferrentd
sudo rm -f /etc/systemd/system/ferrentd.service \
  /usr/local/bin/ferrentd /usr/local/bin/ferrent-cli /usr/local/bin/ferrent-gui
sudo rm -rf /var/lib/ferrent /etc/ferrent
```
