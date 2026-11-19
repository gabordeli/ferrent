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
