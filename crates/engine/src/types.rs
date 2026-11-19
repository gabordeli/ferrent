//! Engine-agnostic value types.

use std::fmt;
use std::path::PathBuf;

/// Stable identifier assigned by the engine to a torrent.
pub type TorrentId = u64;

/// Lifecycle state of a torrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentState {
    Queued,
    Checking,
    Downloading,
    Seeding,
    Paused,
    Error(String),
}

impl fmt::Display for TorrentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => f.write_str("queued"),
            Self::Checking => f.write_str("checking"),
            Self::Downloading => f.write_str("downloading"),
            Self::Seeding => f.write_str("seeding"),
            Self::Paused => f.write_str("paused"),
            Self::Error(message) => write!(f, "error: {message}"),
        }
    }
}

/// Point-in-time snapshot of a torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentStatus {
    pub id: TorrentId,
    pub name: String,
    pub total_bytes: u64,
    pub state: TorrentState,
    /// Completion ratio in `0.0..=1.0`.
    pub progress: f32,
    pub download_rate: u64,
    pub upload_rate: u64,
}

/// Where a new torrent comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentSource {
    /// Raw `.torrent` metainfo bytes.
    Metainfo(Vec<u8>),
    /// Magnet URI.
    Magnet(String),
}

/// Request to add a torrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddTorrent {
    pub name: String,
    /// Known size in bytes, or `0` when the metadata is not available yet.
    pub total_bytes: u64,
    pub source: TorrentSource,
    pub save_path: PathBuf,
}

/// Engine-wide rate limits. A value of `0` means unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineSettings {
    /// Download limit in bytes per second.
    pub download_rate_limit: u64,
    /// Upload limit in bytes per second.
    pub upload_rate_limit: u64,
}

/// Delta emitted by the engine. Consumers apply these to their state mirror.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Added(TorrentStatus),
    StateChanged {
        id: TorrentId,
        state: TorrentState,
    },
    Progress {
        id: TorrentId,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
    },
    Removed {
        id: TorrentId,
    },
}

/// Errors returned by engine operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    NotFound {
        id: TorrentId,
    },
    InvalidTransition {
        id: TorrentId,
        from: TorrentState,
        action: &'static str,
    },
    Rejected {
        reason: String,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "torrent {id} not found"),
            Self::InvalidTransition { id, from, action } => {
                write!(f, "cannot {action} torrent {id} while it is {from}")
            }
            Self::Rejected { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for EngineError {}
