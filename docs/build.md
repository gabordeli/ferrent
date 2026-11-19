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
