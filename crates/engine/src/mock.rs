//! In-memory engine used by unit tests and by the daemon before the
//! `libtorrent` backend is wired in.

use std::collections::BTreeMap;

use tokio::sync::broadcast;

use crate::engine::TorrentEngine;
use crate::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentState, TorrentStatus,
};

const EVENT_BUFFER: usize = 256;

/// Deterministic in-memory [`TorrentEngine`] implementation.
#[derive(Debug)]
pub struct MockEngine {
    next_id: TorrentId,
    torrents: BTreeMap<TorrentId, TorrentStatus>,
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
            settings: EngineSettings::default(),
            events,
        }
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
        let state_change = {
            let status = self
                .torrents
                .get_mut(&id)
                .ok_or(EngineError::NotFound { id })?;
            status.progress = progress;
            status.download_rate = download_rate;
            status.upload_rate = upload_rate;
            if progress >= 1.0 && status.state != TorrentState::Seeding {
                status.state = TorrentState::Seeding;
                Some(status.state.clone())
            } else {
                None
            }
        };

        if let Some(state) = state_change {
            self.emit(EngineEvent::StateChanged { id, state });
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

impl TorrentEngine for MockEngine {
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError> {
        let id = self.next_id;
        self.next_id += 1;

        let status = TorrentStatus {
            id,
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
        };
        self.torrents.insert(id, status.clone());
        self.emit(EngineEvent::Added(status));
        Ok(id)
    }

    fn restore_torrent(&mut self, id: TorrentId, spec: AddTorrent) -> Result<(), EngineError> {
        if self.torrents.contains_key(&id) {
            return Err(EngineError::Rejected {
                reason: format!("torrent {id} already exists"),
            });
        }
        let status = TorrentStatus {
            id,
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
        };
        self.next_id = self.next_id.max(id + 1);
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
            if status.state == TorrentState::Downloading {
                return Ok(());
            }
            status.state = TorrentState::Downloading;
            status.state.clone()
        };
        self.emit(EngineEvent::StateChanged { id, state });
        Ok(())
    }

    fn remove(&mut self, id: TorrentId, delete_files: bool) -> Result<(), EngineError> {
        let _ = delete_files;
        if self.torrents.remove(&id).is_none() {
            return Err(EngineError::NotFound { id });
        }
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
            EngineEvent::Progress { progress, .. } => {
                assert!((progress - 1.0).abs() < 1e-6);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
