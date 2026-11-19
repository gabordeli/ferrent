//! Torrent engine abstraction.
//!
//! The `libtorrent` backend lives behind the `libtorrent` feature so that
//! `ferrent-cli` and `ferrent-gui` can build without a C++ toolchain.

mod engine;
mod mock;
mod types;

pub use engine::TorrentEngine;
pub use mock::MockEngine;
pub use types::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentSource, TorrentState,
    TorrentStatus,
};

/// Version of the engine crate, reported in diagnostics.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
