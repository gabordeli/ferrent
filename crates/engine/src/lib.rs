//! Torrent engine abstraction.
//!
//! The `libtorrent` backend lives behind the `libtorrent` feature so that
//! `ferrent-cli` and `ferrent-gui` can build without a C++ toolchain.

/// Version of the engine crate, reported in diagnostics.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
