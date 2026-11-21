//! The engine contract.

use std::path::PathBuf;

use tokio::sync::broadcast;

use crate::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, QueueDirection, TorrentFileInfo,
    TorrentId, TorrentLimits, TorrentPeerInfo, TorrentStatus, TorrentTrackerInfo,
};

/// Synchronous torrent engine contract.
///
/// Implementations own their worker thread (or use a blocking session) and must
/// not block an async runtime worker. The daemon talks to the engine from a
/// dedicated thread and forwards [`EngineEvent`]s to its state mirror.
pub trait TorrentEngine: Send + 'static {
    /// Adds a torrent and returns its engine-assigned id.
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError>;

    /// Restores a torrent with a stable id (used by the daemon on startup).
    fn restore_torrent(&mut self, id: TorrentId, spec: AddTorrent) -> Result<(), EngineError>;

    /// Requests fastresume data for a torrent.
    fn request_resume_data(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Takes previously requested fastresume data, if it is ready.
    fn take_resume_data(&mut self, id: TorrentId) -> Result<Option<Vec<u8>>, EngineError>;

    /// Pauses a torrent. Idempotent when it is already paused.
    fn pause(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Resumes a torrent. Idempotent when it is already downloading.
    fn resume(&mut self, id: TorrentId) -> Result<(), EngineError>;

    /// Applies per-torrent rate and connection limits.
    fn set_torrent_limits(
        &mut self,
        id: TorrentId,
        limits: TorrentLimits,
    ) -> Result<(), EngineError>;

    /// Moves the payload to a new save path.
    ///
    /// Implementations set [`crate::TorrentState::Moving`] while the move is in
    /// progress and return to the previous state when it completes.
    fn move_storage(&mut self, id: TorrentId, path: PathBuf) -> Result<(), EngineError>;

    /// Pauses every torrent in `ids`; returns how many transitions succeeded.
    fn pause_many(&mut self, ids: &[TorrentId]) -> usize {
        ids.iter().filter(|&&id| self.pause(id).is_ok()).count()
    }

    /// Resumes every torrent in `ids`; returns how many transitions succeeded.
    fn resume_many(&mut self, ids: &[TorrentId]) -> usize {
        ids.iter().filter(|&&id| self.resume(id).is_ok()).count()
    }

    /// Removes a torrent and its engine state.
    ///
    /// With `delete_files` the torrent's payload is deleted from its save path.
    fn remove(&mut self, id: TorrentId, delete_files: bool) -> Result<(), EngineError>;

    /// Moves a torrent one slot up or down in the download queue.
    fn move_in_queue(
        &mut self,
        id: TorrentId,
        direction: QueueDirection,
    ) -> Result<(), EngineError>;

    /// Returns a snapshot of every torrent, sorted by id.
    fn list(&self) -> Vec<TorrentStatus>;

    /// Returns one torrent snapshot.
    fn status(&self, id: TorrentId) -> Result<TorrentStatus, EngineError>;

    /// Returns the file list of one torrent.
    fn files(&mut self, id: TorrentId) -> Result<Vec<TorrentFileInfo>, EngineError> {
        let _ = id;
        Ok(Vec::new())
    }

    /// Replaces the per-file priorities (0 skip, 1 low, 4 normal, 7 high).
    ///
    /// The slice holds one entry per file, in the order of [`Self::files`].
    fn set_file_priorities(
        &mut self,
        id: TorrentId,
        priorities: &[i32],
    ) -> Result<(), EngineError> {
        let _ = (id, priorities);
        Ok(())
    }

    /// Returns the connected peers of one torrent.
    fn peers(&mut self, id: TorrentId) -> Result<Vec<TorrentPeerInfo>, EngineError> {
        let _ = id;
        Ok(Vec::new())
    }

    /// Returns the trackers of one torrent.
    fn trackers(&mut self, id: TorrentId) -> Result<Vec<TorrentTrackerInfo>, EngineError> {
        let _ = id;
        Ok(Vec::new())
    }

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
