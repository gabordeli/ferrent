//! In-memory engine used by unit tests and by the daemon before the
//! `libtorrent` backend is wired in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use tokio::sync::broadcast;

use crate::engine::TorrentEngine;
use crate::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentLimits, TorrentSource,
    TorrentState, TorrentStatus,
};

const EVENT_BUFFER: usize = 256;

/// Deterministic in-memory [`TorrentEngine`] implementation.
#[derive(Debug)]
pub struct MockEngine {
    next_id: TorrentId,
    torrents: BTreeMap<TorrentId, TorrentStatus>,
    infohash_index: BTreeMap<String, TorrentId>,
    limits: BTreeMap<TorrentId, TorrentLimits>,
    finished: BTreeSet<TorrentId>,
    settings: EngineSettings,
    events: broadcast::Sender<EngineEvent>,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MockEngine {
    /// Creates an empty engine.
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            next_id: 0,
            torrents: BTreeMap::new(),
            infohash_index: BTreeMap::new(),
            limits: BTreeMap::new(),
            finished: BTreeSet::new(),
            settings: EngineSettings::default(),
            events,
        }
    }

    /// Test helper: the limits currently applied to a torrent.
    #[must_use]
    pub fn limits(&self, id: TorrentId) -> Option<TorrentLimits> {
        self.limits.get(&id).copied()
    }

    /// Test helper: advances progress and emits the resulting events.
    pub fn advance(
        &mut self,
        id: TorrentId,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
    ) -> Result<(), EngineError> {
        let progress = progress.clamp(0.0, 1.0);
        let (state_change, finished) = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            status.progress = progress;
            status.download_rate = download_rate;
            status.upload_rate = upload_rate;
            let state_change = if progress >= 1.0 && status.state != TorrentState::Seeding {
                status.state = TorrentState::Seeding;
                Some(status.state.clone())
            } else {
                None
            };
            let finished = progress >= 1.0 && self.finished.insert(id);
            (state_change, finished)
        };

        if let Some(state) = state_change {
            self.emit(EngineEvent::StateChanged { id, state });
        }
        if finished {
            self.emit(EngineEvent::Finished { id });
        }
        self.emit(EngineEvent::Progress {
            id,
            progress,
            download_rate,
            upload_rate,
            queue_position: 0,
        });
        Ok(())
    }

    fn emit(&self, event: EngineEvent) {
        let _ = self.events.send(event);
    }
}

fn status_from_spec(id: TorrentId, spec: AddTorrent, infohash: String) -> TorrentStatus {
    TorrentStatus {
        id,
        infohash,
        name: spec.name,
        save_path: spec.save_path,
        total_bytes: spec.total_bytes,
        state: TorrentState::Queued,
        progress: 0.0,
        download_rate: 0,
        upload_rate: 0,
        seeds: 0,
        peers: 0,
        eta_seconds: 0,
        queue_position: 0,
    }
}

/// Stable mock identity; not a real SHA-1.
fn infohash_for(spec: &AddTorrent) -> String {
    if let TorrentSource::Magnet(uri) = &spec.source {
        if let Some(rest) = uri.split("urn:btih:").nth(1) {
            let hash = rest.split('&').next().unwrap_or(rest);
            if hash.len() == 40 && hash.chars().all(|character| character.is_ascii_hexdigit()) {
                return hash.to_ascii_lowercase();
            }
        }
    }
    let bytes = match &spec.source {
        TorrentSource::Metainfo(bytes) => bytes.as_slice(),
        TorrentSource::Magnet(uri) => uri.as_bytes(),
    };
    synthetic_infohash(bytes)
}

fn synthetic_infohash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!(
        "{hash:016x}{:016x}{:08x}",
        hash.rotate_left(21),
        hash.rotate_left(43) as u32
    )
}

impl TorrentEngine for MockEngine {
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError> {
        let infohash = infohash_for(&spec);
        if let Some(existing) = self.infohash_index.get(&infohash) {
            return Err(EngineError::AlreadyExists { id: *existing });
        }
        let id = self.next_id;
        self.next_id += 1;
        let status = status_from_spec(id, spec, infohash.clone());
        self.infohash_index.insert(infohash, id);
        self.torrents.insert(id, status.clone());
        self.emit(EngineEvent::Added(status));
        Ok(id)
    }

    fn restore_torrent(&mut self, id: TorrentId, spec: AddTorrent) -> Result<(), EngineError> {
        if self.torrents.contains_key(&id) {
            return Err(EngineError::AlreadyExists { id });
        }
        let infohash = infohash_for(&spec);
        if let Some(existing) = self.infohash_index.get(&infohash) {
            return Err(EngineError::AlreadyExists { id: *existing });
        }
        let status = status_from_spec(id, spec, infohash.clone());
        self.next_id = self.next_id.max(id + 1);
        self.infohash_index.insert(infohash, id);
        self.torrents.insert(id, status.clone());
        self.emit(EngineEvent::Added(status));
        Ok(())
    }

    fn request_resume_data(&mut self, id: TorrentId) -> Result<(), EngineError> {
        if self.torrents.contains_key(&id) {
            Ok(())
        } else {
            Err(EngineError::NotFound { id })
        }
    }

    fn take_resume_data(&mut self, id: TorrentId) -> Result<Option<Vec<u8>>, EngineError> {
        if self.torrents.contains_key(&id) {
            Ok(Some(b"mock-resume".to_vec()))
        } else {
            Ok(None)
        }
    }

    fn pause(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let state = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            if status.state == TorrentState::Paused {
                return Ok(());
            }
            status.state = TorrentState::Paused;
            status.download_rate = 0;
            status.upload_rate = 0;
            status.state.clone()
        };
        self.emit(EngineEvent::StateChanged { id, state });
        Ok(())
    }

    fn resume(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let state = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            if status.state == TorrentState::Downloading || status.state == TorrentState::Seeding {
                return Ok(());
            }
            status.state = if status.progress >= 1.0 {
                TorrentState::Seeding
            } else {
                TorrentState::Downloading
            };
            status.state.clone()
        };
        self.emit(EngineEvent::StateChanged { id, state });
        Ok(())
    }

    fn set_torrent_limits(
        &mut self,
        id: TorrentId,
        limits: TorrentLimits,
    ) -> Result<(), EngineError> {
        if !self.torrents.contains_key(&id) {
            return Err(EngineError::NotFound { id });
        }
        self.limits.insert(id, limits);
        Ok(())
    }

    fn move_storage(&mut self, id: TorrentId, path: PathBuf) -> Result<(), EngineError> {
        let previous = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            if status.state == TorrentState::Moving {
                return Ok(());
            }
            let previous = status.state.clone();
            status.state = TorrentState::Moving;
            status.download_rate = 0;
            status.upload_rate = 0;
            previous
        };
        self.emit(EngineEvent::StateChanged {
            id,
            state: TorrentState::Moving,
        });

        let state = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            status.save_path = path;
            status.state = match previous {
                TorrentState::Paused => TorrentState::Paused,
                TorrentState::Queued => TorrentState::Queued,
                TorrentState::Error(message) => TorrentState::Error(message),
                _ if status.progress >= 1.0 => TorrentState::Seeding,
                _ => TorrentState::Downloading,
            };
            status.state.clone()
        };
        self.emit(EngineEvent::StateChanged { id, state });
        Ok(())
    }

    fn remove(&mut self, id: TorrentId, delete_files: bool) -> Result<(), EngineError> {
        let _ = delete_files;
        let status = self
            .torrents
            .remove(&id)
            .ok_or(EngineError::NotFound { id })?;
        self.infohash_index.remove(&status.infohash);
        self.limits.remove(&id);
        self.finished.remove(&id);
        self.emit(EngineEvent::Removed { id });
        Ok(())
    }

    fn move_in_queue(
        &mut self,
        id: TorrentId,
        direction: crate::QueueDirection,
    ) -> Result<(), EngineError> {
        let _ = direction;
        if !self.torrents.contains_key(&id) {
            return Err(EngineError::NotFound { id });
        }
        Ok(())
    }

    fn list(&self) -> Vec<TorrentStatus> {
        self.torrents.values().cloned().collect()
    }

    fn status(&self, id: TorrentId) -> Result<TorrentStatus, EngineError> {
        self.torrents
            .get(&id)
            .cloned()
            .ok_or(EngineError::NotFound { id })
    }

    fn settings(&self) -> EngineSettings {
        self.settings
    }

    fn set_settings(&mut self, settings: EngineSettings) {
        self.settings = settings;
    }

    fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::TorrentSource;

    fn add(engine: &mut MockEngine, name: &str) -> TorrentId {
        engine
            .add_torrent(AddTorrent {
                name: name.to_owned(),
                total_bytes: 1024,
                source: TorrentSource::Magnet(format!("magnet:?xt=urn:btih:{name}")),
                save_path: PathBuf::from("/tmp/ferrent"),
                resume_data: None,
            })
            .expect("add torrent")
    }

    fn recv(events: &mut broadcast::Receiver<EngineEvent>) -> EngineEvent {
        events.try_recv().expect("expected an engine event")
    }

    #[test]
    fn add_assigns_ids_and_emits_added() {
        let mut engine = MockEngine::new();
        let mut events = engine.subscribe();

        let first = add(&mut engine, "first");
        let second = add(&mut engine, "second");

        assert_eq!(first, 0);
        assert_eq!(second, 1);
        assert_eq!(engine.list().len(), 2);

        match recv(&mut events) {
            EngineEvent::Added(status) => {
                assert_eq!(status.id, first);
                assert_eq!(status.name, "first");
                assert_eq!(status.state, TorrentState::Queued);
                assert_eq!(status.progress, 0.0);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        match recv(&mut events) {
            EngineEvent::Added(status) => assert_eq!(status.id, second),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn pause_and_resume_emit_state_changes() {
        let mut engine = MockEngine::new();
        let mut events = engine.subscribe();
        let id = add(&mut engine, "download");
        let _ = recv(&mut events);

        engine.pause(id).expect("pause");
        match recv(&mut events) {
            EngineEvent::StateChanged {
                id: event_id,
                state,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(state, TorrentState::Paused);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(
            engine.status(id).expect("status").state,
            TorrentState::Paused
        );

        engine.pause(id).expect("idempotent pause");
        assert!(events.try_recv().is_err());

        engine.resume(id).expect("resume");
        match recv(&mut events) {
            EngineEvent::StateChanged {
                id: event_id,
                state,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(state, TorrentState::Downloading);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn remove_emits_removed_and_status_is_gone() {
        let mut engine = MockEngine::new();
        let mut events = engine.subscribe();
        let id = add(&mut engine, "remove-me");
        let _ = recv(&mut events);

        engine.remove(id, true).expect("remove");
        match recv(&mut events) {
            EngineEvent::Removed { id: event_id } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }
        assert_eq!(engine.status(id), Err(EngineError::NotFound { id }));
    }

    #[test]
    fn list_is_sorted_by_id() {
        let mut engine = MockEngine::new();
        let first = add(&mut engine, "first");
        let second = add(&mut engine, "second");
        let third = add(&mut engine, "third");

        let ids: Vec<TorrentId> = engine.list().into_iter().map(|status| status.id).collect();
        assert_eq!(ids, vec![first, second, third]);
    }

    #[test]
    fn unknown_id_returns_not_found() {
        let mut engine = MockEngine::new();
        let error = engine.pause(42).expect_err("unknown torrent");
        assert_eq!(error, EngineError::NotFound { id: 42 });
    }

    #[test]
    fn advance_emits_progress_and_finishes_seeding() {
        let mut engine = MockEngine::new();
        let mut events = engine.subscribe();
        let id = add(&mut engine, "download");
        let _ = recv(&mut events);

        engine.advance(id, 0.5, 2048, 128).expect("advance");
        match recv(&mut events) {
            EngineEvent::Progress {
                id: event_id,
                progress,
                download_rate,
                upload_rate,
                ..
            } => {
                assert_eq!(event_id, id);
                assert!((progress - 0.5).abs() < 1e-6);
                assert_eq!(download_rate, 2048);
                assert_eq!(upload_rate, 128);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        engine.advance(id, 1.5, 0, 256).expect("finish");
        match recv(&mut events) {
            EngineEvent::StateChanged {
                id: event_id,
                state,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(state, TorrentState::Seeding);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match recv(&mut events) {
            EngineEvent::Finished { id: event_id } => assert_eq!(event_id, id),
            other => panic!("unexpected event: {other:?}"),
        }
        match recv(&mut events) {
            EngineEvent::Progress { progress, .. } => {
                assert!((progress - 1.0).abs() < 1e-6);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        engine.advance(id, 1.0, 0, 256).expect("still finished");
        match recv(&mut events) {
            EngineEvent::Progress { progress, .. } => {
                assert!((progress - 1.0).abs() < 1e-6);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        assert!(events.try_recv().is_err(), "Finished must fire once");
    }

    #[test]
    fn duplicate_add_returns_the_existing_id() {
        let mut engine = MockEngine::new();
        let first = add(&mut engine, "same");
        let error = engine
            .add_torrent(AddTorrent {
                name: "same".to_owned(),
                total_bytes: 1024,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:same".to_owned()),
                save_path: PathBuf::from("/tmp/ferrent"),
                resume_data: None,
            })
            .expect_err("duplicate");
        assert_eq!(error, EngineError::AlreadyExists { id: first });
    }

    #[test]
    fn move_storage_updates_the_path_and_restores_the_state() {
        let mut engine = MockEngine::new();
        let mut events = engine.subscribe();
        let id = add(&mut engine, "move");
        let _ = recv(&mut events);
        engine.resume(id).expect("resume");
        let _ = recv(&mut events);

        engine
            .move_storage(id, PathBuf::from("/mnt/new"))
            .expect("move");
        match recv(&mut events) {
            EngineEvent::StateChanged {
                id: event_id,
                state,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(state, TorrentState::Moving);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        match recv(&mut events) {
            EngineEvent::StateChanged {
                id: event_id,
                state,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(state, TorrentState::Downloading);
            }
            other => panic!("unexpected event: {other:?}"),
        }
        let status = engine.status(id).expect("status");
        assert_eq!(status.save_path, PathBuf::from("/mnt/new"));
    }

    #[test]
    fn bulk_pause_and_resume() {
        let mut engine = MockEngine::new();
        let first = add(&mut engine, "bulk-first");
        let second = add(&mut engine, "bulk-second");

        assert_eq!(engine.pause_many(&[first, second]), 2);
        assert_eq!(
            engine.status(first).expect("first").state,
            TorrentState::Paused
        );
        assert_eq!(
            engine.status(second).expect("second").state,
            TorrentState::Paused
        );

        assert_eq!(engine.resume_many(&[first, second]), 2);
        assert_eq!(
            engine.status(first).expect("first").state,
            TorrentState::Downloading
        );
    }

    #[test]
    fn per_torrent_limits_are_stored() {
        let mut engine = MockEngine::new();
        let id = add(&mut engine, "limits");
        let limits = TorrentLimits {
            download_rate_limit: 1024,
            upload_rate_limit: 512,
            connections_limit: 20,
            upload_slots_limit: 4,
        };
        engine.set_torrent_limits(id, limits).expect("set limits");
        assert_eq!(engine.limits(id), Some(limits));
        assert_eq!(
            engine.set_torrent_limits(99, limits),
            Err(EngineError::NotFound { id: 99 })
        );
    }
}
