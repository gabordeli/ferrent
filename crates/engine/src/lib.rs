//! Torrent engine abstraction.
//!
//! The `libtorrent` backend lives behind the `libtorrent` feature so that
//! `ferrent-cli` and `ferrent-gui` can build without a C++ toolchain.

mod engine;
#[cfg(feature = "libtorrent")]
#[allow(unsafe_code)]
mod libtorrent;
mod mock;
mod types;

pub use engine::TorrentEngine;
#[cfg(feature = "libtorrent")]
pub use libtorrent::{LibtorrentEngine, make_torrent};
pub use mock::MockEngine;
pub use types::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, QueueDirection, TorrentFileInfo,
    TorrentId, TorrentLimits, TorrentPeerInfo, TorrentSource, TorrentState, TorrentStatus,
    TorrentTrackerInfo,
};

/// Version of the engine crate, reported in diagnostics.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
