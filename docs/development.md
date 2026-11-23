# Build és teszt

## Követelmények

- Rust stable. A `rust-toolchain.toml` a stable channelt és a `rustfmt`/`clippy`
  componenteket kéri; rustup-pal a legegyszerűbb.
- Debian/Ubuntu, a `libtorrent` backendhez:

  ```bash
  sudo apt-get install -y build-essential pkg-config libtorrent-rasterbar-dev libboost-dev
  ```

## Parancsok

A C++ eszközláncot nem igénylő alap workspace:

```bash
cargo build --workspace
cargo test --workspace
```

A valódi torrent motor:

```bash
cargo build -p ferrent-engine --features libtorrent
cargo test -p ferrent-engine --features libtorrent
```

A teljes helyi kapu (a `libtorrent` feature nélkül):

```bash
./ci/check.sh
```

A `libtorrent` backend kapuja:

```bash
./ci/check-libtorrent.sh
```

## GUI build

A `ferrent-gui` az `iced` 0.13-at használja; a rendszerkönyvtárak:

```bash
# Debian 12/13, Ubuntu 24.04
sudo apt-get install -y libxkbcommon-dev libwayland-dev libx11-dev \
  libxcursor-dev libxrandr-dev libxi-dev libglib2.0-dev libgtk-3-dev

# Fedora
sudo dnf install -y libxkbcommon-devel wayland-devel libX11-devel \
  libXcursor-devel libXrandr-devel libXi-devel glib2-devel gtk3-devel
```

```bash
cargo build -p ferrent-gui
```

Fej nélküli (Xvfb) ellenőrzéshez `ICED_BACKEND=tiny-skia`, és a Wayland
kikapcsolása kell: `env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 …`;
a teljes, mért recept (xauth, ablak-offset a kattintásnál, teljes
felbontású crop): [docs/headless-gui.md](headless-gui.md).

## Demo adatok

```bash
cargo build --release -p ferrent-gui --example demo_server
./target/release/examples/demo_server --torrents 4000 --port 50129 \
  --data-dir /tmp/ferrent-demo
```

A GUI profil kitöltése és a CLI-vel való kipróbálás: `docs/demo.md`.

## Store (M1-T1)

```bash
cargo test -p ferrent-daemon
```

A store tesztek redb felett futnak; a `crash_recovery` teszt a
`crash_writer` bin segítségével SIGKILL-t küld egy nyitott write tranzakcióra,
majd ellenőrzi, hogy az adatbázis újranyitható és a korábbi commit megvan.
A 100 ms-os batch-cél a release buildre vonatkozik:

```bash
cargo test -p ferrent-daemon --release -- --nocapture
```

A `crash_recovery` csak Unixon fut (SIGKILL).

## Daemon persistence (M1-T5)

A `ferrentd` a `FERRENT_DATA_DIR` alatt tartja a `state.redb`-t és a TLS
anyagot. Indításkor a torrent rekordokat (metainfo, mentési út, fastresume)
betölti és restore-olja; `SIGTERM`/`SIGINT` esetén fastresume-t ment.
`kill -9` esetén az utolsó fastresume elavulhat, ilyenkor a libtorrent
recheckelhet. A részletes lépések: `docs/cli.md`.

## RPC / TLS (M1-T3)

```bash
cargo test -p ferrent-daemon --test rpc_tls
```

- A szerver önaláírt certet generál (`TlsMaterial`); a kliens ezt a certet
  használja CA-ként (pin). A fingerprint SHA-256.
- A token `authorization: Bearer <token>` fejlécben megy; rossz token →
  `UNAUTHENTICATED`.
- A proto: `crates/protocol/proto/ferrent/v1/ferrent.proto`; a `protoc`ot a
  `protoc-bin-vendored` adja, nem kell rendszer csomag.
- Buf lint/breaking (Dockerrel):

  ```bash
  docker run --rm -v "$PWD:/workspace" -w /workspace bufbuild/buf:latest lint
  ```

## CLI és daemon (M1-T4)

```bash
cargo run -p ferrent-cli -- --help
cargo test -p ferrent-cli
```

A CLI teszt in-process daemont indít, így nincs szükség külön szerverre.
A futtatható daemon:

```bash
FERRENT_TOKEN=secret FERRENT_DATA_DIR=./ferrent-data \
cargo run -p ferrent-daemon
```

Valódi torrent motorral:

```bash
cargo run -p ferrent-daemon --features libtorrent
```

A részletes CLI/konfig/JSON/exit-code leírás: `docs/cli.md`.

## Load harness (M0-T4)

```bash
cargo run -p ferrent-engine --example loadtest --features libtorrent -- \
    --torrents 4000 --mode seed
```

A riport a `docs/measurements/m0-loadtest.json` és
`docs/measurements/m0-loadtest.md` fájlokba kerül. A harness 4 000 egyedi,
256 bájtos torrentet hoz létre a libtorrent saját encoderével, seed módban,
10 s settle és 10 s idle mérés mellett. A harness nem része a gyors CI
kapunak; kézzel futtatandó.

## Megjegyzések

- A `libtorrent-rasterbar-dev` a fejléceket és a linkelhető libraryt adja; a
  futásidejű library (`libtorrent-rasterbar2.0t64` Debianon) külön csomag.
- Debian 13 (trixie): libtorrent 2.0.11. Debian 12: 2.0.x. A bridge csak a
  2.0 sorozaton stabil API-t használ (lásd `docs/adr/0001-libtorrent-version.md`).
- A `cxx` bridge a `crates/engine/build.rs`-ben fordul, C++17-tel. A C++
  session a `ferrent-libtorrent` nevű dedikált szálon fut.
- A teszt fixture egy valódi, 258 bájtos `.torrent`:
  `crates/engine/tests/fixtures/hello.torrent` (14 KiB `payload.bin`, 16 KiB
  piece). A teszt a payloadot temp könyvtárba írja, így hálózat nélkül tudja
  ellenőrizni az add/pause/resume/remove útvonalat.
