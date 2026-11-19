//! The engine contract.

use tokio::sync::broadcast;

use crate::{AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentStatus};

/// Synchronous torrent engine contract.
///
/// Implementations own their worker thread (or use a blocking session) and must
/// not block an async runtime worker. The daemon talks to the engine from a
/// dedicated thread and forwards [`EngineEvent`]s to its state mirror.
pub trait TorrentEngine: Send + 'static {
    /// Adds a torrent and returns its engine-assigned id.
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError>;

    /// Pauses a torrent. Idempotent when it is already paused.
    fn pause(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Resumes a torrent. Idempotent when it is already downloading.
    fn resume(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Removes a torrent and its engine state.
    fn remove(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Returns a snapshot of every torrent, sorted by id.
    fn list(&self) -> Vec<TorrentStatus>;

    /// Returns one torrent snapshot.
    fn status(&self, id: TorrentId) -> Result<TorrentStatus, EngineError>;

    /// Returns the current engine settings.
    fn settings(&self) -> EngineSettings;

    /// Replaces the engine settings.
    fn set_settings(&mut self, settings: EngineSettings);

    /// Subscribes to engine events.
    ///
    /// Delivery is best-effort: a subscriber that falls behind may lag rather
    /// than block the engine.
    fn subscribe(&self) -> broadcast::Receiver<EngineEvent>;
}
