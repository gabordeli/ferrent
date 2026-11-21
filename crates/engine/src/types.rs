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
    /// The engine is moving the payload to a new save path.
    Moving,
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
            Self::Moving => f.write_str("moving"),
            Self::Error(message) => write!(f, "error: {message}"),
        }
    }
}

/// Point-in-time snapshot of a torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentStatus {
    pub id: TorrentId,
    /// v1 info hash as 40 lowercase hex characters.
    pub infohash: String,
    pub name: String,
    /// Directory the torrent's files are written to, as the engine resolved it.
    pub save_path: PathBuf,
    pub total_bytes: u64,
    pub state: TorrentState,
    /// Completion ratio in `0.0..=1.0`.
    pub progress: f32,
    pub download_rate: u64,
    pub upload_rate: u64,
    /// Payload bytes uploaded since the torrent was added (all-time).
    pub uploaded_bytes: u64,
    /// Payload bytes downloaded since the torrent was added (all-time).
    pub downloaded_bytes: u64,
    /// Connected seeds reported by the engine.
    pub seeds: u32,
    /// Connected peers reported by the engine.
    pub peers: u32,
    /// Estimated seconds to completion; zero when unknown.
    pub eta_seconds: u32,
    /// Position in the download queue; negative when the engine has none.
    pub queue_position: i32,
}

/// Direction of a queue move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueDirection {
    Up,
    Down,
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
    /// Fastresume data returned by [`crate::TorrentEngine::take_resume_data`].
    pub resume_data: Option<Vec<u8>>,
}

/// Engine-wide rate limits. A value of `0` means unlimited.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineSettings {
    /// Download limit in bytes per second.
    pub download_rate_limit: u64,
    /// Upload limit in bytes per second.
    pub upload_rate_limit: u64,
    /// Treat newly added torrents as complete without an initial file check.
    pub seed_mode: bool,
    /// Maximum number of simultaneously downloading torrents; `0` uses the
    /// engine's 20 000 soak head-room.
    pub active_downloads: u32,
    /// Maximum number of simultaneously seeding torrents; `0` uses the engine's
    /// 20 000 soak head-room.
    pub active_seeds: u32,
    /// Maximum number of active torrents; `0` uses the engine's 20 000 soak
    /// head-room.
    pub active_limit: u32,
    /// libtorrent `listen_interfaces` (e.g. `0.0.0.0:6881`); empty leaves the
    /// engine default.
    pub listen_interfaces: String,
    /// libtorrent `outgoing_interfaces` (e.g. `wg0/10.8.0.2`); empty means
    /// "any".
    pub outgoing_interfaces: String,
    /// Trackers added to torrents whose metainfo has no announce URL.
    pub default_trackers: Vec<String>,
}

/// One torrent-creation request, resolved on the engine host.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateTorrentSpec {
    /// File or directory to publish.
    pub path: std::path::PathBuf,
    /// Piece size in bytes; `0` picks a 256 KiB default.
    pub piece_length: u32,
    pub trackers: Vec<String>,
}

/// The resulting metainfo and its v1 info hash.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedTorrent {
    pub metainfo: Vec<u8>,
    pub infohash: String,
}

/// Per-torrent limits. A value of `0` means unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TorrentLimits {
    /// Download limit in bytes per second.
    pub download_rate_limit: u64,
    /// Upload limit in bytes per second.
    pub upload_rate_limit: u64,
    /// Maximum number of peer connections.
    pub connections_limit: u32,
    /// Maximum number of unchoked upload slots.
    pub upload_slots_limit: u32,
}

/// Delta emitted by the engine. Consumers apply these to their state mirror.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    Added(TorrentStatus),
    StateChanged {
        id: TorrentId,
        state: TorrentState,
    },
    /// The torrent reached 100% and fired exactly once per engine lifetime.
    Finished {
        id: TorrentId,
    },
    Progress {
        id: TorrentId,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
        queue_position: i32,
        /// Connected seeds, as the engine sees them.
        seeds: i32,
        /// Connected peers (leechers), as the engine sees them.
        peers: i32,
    },
    Removed {
        id: TorrentId,
    },
}

/// Errors returned by engine operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    AlreadyExists {
        id: TorrentId,
    },
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
            Self::AlreadyExists { id } => write!(f, "torrent {id} already exists"),
            Self::NotFound { id } => write!(f, "torrent {id} not found"),
            Self::InvalidTransition { id, from, action } => {
                write!(f, "cannot {action} torrent {id} while it is {from}")
            }
            Self::Rejected { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for EngineError {}

/// One file inside a torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentFileInfo {
    pub path: String,
    pub size: u64,
    pub progress: f32,
    pub priority: i32,
}

/// One connected peer.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentPeerInfo {
    pub endpoint: String,
    pub client: String,
    pub progress: f32,
    pub download_rate: u64,
    pub upload_rate: u64,
}

/// One tracker announce entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentTrackerInfo {
    pub url: String,
    /// Machine-readable announce state: `working`, `updating`, `unverified` or
    /// `error`.
    pub status: String,
    pub seeds: u32,
    pub leechers: u32,
    /// The tracker's own message, or the announce error text.
    pub message: String,
    /// Seconds until the next announce (0 when unknown).
    pub next_announce: u32,
    /// Consecutive failed announce attempts.
    pub fails: u32,
}
