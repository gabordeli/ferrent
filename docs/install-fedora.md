# Telepítés Fedora alá

Fedora-csomag (RPM) egyelőre nincs; a daemon, a CLI és a GUI forrásból
épül. A lépések Fedora 40/41-en működnek, x86_64-en.

## 1. Függőségek

```bash
sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config \
  libtorrent-rasterbar-devel boost-devel openssl-devel
```

A GUI-hoz (opcionális):

```bash
sudo dnf install -y libxkbcommon-devel wayland-devel libX11-devel \
  libXcursor-devel libXrandr-devel libXi-devel glib2-devel gtk3-devel
```

Rust: <https://rustup.rs> (a `rust-toolchain.toml` a stable channelt kéri).

## 2. Build

```bash
git clone https://cirice-gitlab.ddns.net/gabor/ferrent.git
cd ferrent
cargo build --release -p ferrent-daemon --features libtorrent
cargo build --release -p ferrent-cli -p ferrent-gui
```

## 3. Futtatás

```bash
sudo install -d -m 0755 /var/lib/ferrent /etc/ferrent
sudo install -m 0755 target/release/ferrentd target/release/ferrent-cli /usr/local/bin/

FERRENT_DATA_DIR=/var/lib/ferrent \
FERRENT_TOKEN="$(head -c 24 /dev/urandom | base64)" \
FERRENT_BIND=127.0.0.1:50051 \
/usr/local/bin/ferrentd
```

A token a kimenetben megjelenő érték; a daemon a `tls-cert.pem`-et a
`FERRENT_DATA_DIR` alá írja. A CLI:

```bash
ferrent-cli --server https://localhost:50051 \
  --ca /var/lib/ferrent/tls-cert.pem --token '<token>' status
```

## 4. systemd (opcionális)

```bash
sudo tee /etc/ferrent/ferrentd.env >/dev/null <<'EOF'
FERRENT_DATA_DIR=/var/lib/ferrent
FERRENT_BIND=127.0.0.1:50051
EOF
sudo tee /etc/systemd/system/ferrentd.service >/dev/null <<'EOF'
[Unit]
Description=Ferrent BitTorrent daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ferrent
Group=ferrent
EnvironmentFile=/etc/ferrent/ferrentd.env
ExecStart=/usr/local/bin/ferrentd
Restart=on-failure
StateDirectory=ferrent
PrivateTmp=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/ferrent

[Install]
WantedBy=multi-user.target
EOF
sudo useradd --system --home /var/lib/ferrent --shell /usr/sbin/nologin ferrent
echo '<token>' | sudo tee /etc/ferrent/token >/dev/null
sudo chown root:ferrent /etc/ferrent/token && sudo chmod 0640 /etc/ferrent/token
sudo systemctl daemon-reload && sudo systemctl enable --now ferrentd
```

A tokent a `token_file` config olvassa (alap:
`/etc/ferrent/token`); ugyanez az érték kell a klienseknek. A `.deb`
telepítés ezt a postinstben generálja, itt kézzel kell.

## Eltávolítás

```bash
sudo systemctl disable --now ferrentd
sudo rm -f /etc/systemd/system/ferrentd.service /usr/local/bin/ferrentd /usr/local/bin/ferrent-cli
sudo rm -rf /var/lib/ferrent /etc/ferrent
```
