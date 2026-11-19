//! `libtorrent` backend. The C++ session lives on a dedicated worker thread;
//! the Rust side talks to it over a command channel and receives deltas on a
//! broadcast channel.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::engine::TorrentEngine;
use crate::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentSource, TorrentState,
    TorrentStatus,
};

use self::bridge::ffi::{FfiEvent, FfiStatus, new_session};

mod bridge;

/// Builds a single-file v1 metainfo blob; used by the M0 load harness.
#[must_use]
pub fn make_torrent(name: &str, payload: &[u8]) -> Vec<u8> {
    bridge::ffi::make_torrent(name, payload)
}

const EVENT_BUFFER: usize = 8192;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

enum Command {
    Add {
        spec: AddTorrent,
        reply: Sender<Result<TorrentId, EngineError>>,
    },
    Restore {
        id: TorrentId,
        spec: AddTorrent,
        reply: Sender<Result<(), EngineError>>,
    },
    RequestResume {
        id: TorrentId,
        reply: Sender<Result<(), EngineError>>,
    },
    TakeResume {
        id: TorrentId,
        reply: Sender<Result<Option<Vec<u8>>, EngineError>>,
    },
    Pause {
        id: TorrentId,
        reply: Sender<Result<(), EngineError>>,
    },
    Resume {
        id: TorrentId,
        reply: Sender<Result<(), EngineError>>,
    },
    Remove {
        id: TorrentId,
        reply: Sender<Result<(), EngineError>>,
    },
    Files {
        id: TorrentId,
        reply: Sender<Result<Vec<crate::TorrentFileInfo>, EngineError>>,
    },
    Peers {
        id: TorrentId,
        reply: Sender<Result<Vec<crate::TorrentPeerInfo>, EngineError>>,
    },
    Trackers {
        id: TorrentId,
        reply: Sender<Result<Vec<crate::TorrentTrackerInfo>, EngineError>>,
    },
    List {
        reply: Sender<Vec<TorrentStatus>>,
    },
    Status {
        id: TorrentId,
        reply: Sender<Result<TorrentStatus, EngineError>>,
    },
    SetSettings {
        settings: EngineSettings,
        reply: Sender<()>,
    },
    Shutdown,
}

/// `libtorrent`-backed engine.
pub struct LibtorrentEngine {
    commands: Sender<Command>,
    events: broadcast::Sender<EngineEvent>,
    worker: Option<JoinHandle<()>>,
    settings: EngineSettings,
}

impl LibtorrentEngine {
    /// Starts the worker thread and creates the libtorrent session.
    #[must_use]
    pub fn new(settings: EngineSettings) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let (commands, command_rx) = mpsc::channel();
        let worker_events = events.clone();
        let worker = thread::Builder::new()
            .name("ferrent-libtorrent".to_owned())
            .spawn(move || run_worker(command_rx, worker_events, settings))
            .expect("spawn libtorrent worker thread");

        Self {
            commands,
            events,
            worker: Some(worker),
            settings,
        }
    }

    fn send(&self, command: Command) -> Result<(), EngineError> {
        self.commands
            .send(command)
            .map_err(|_| EngineError::Rejected {
                reason: "libtorrent worker stopped".to_owned(),
            })
    }

    fn recv<T>(reply: Receiver<Result<T, EngineError>>) -> Result<T, EngineError> {
        reply.recv().map_err(|_| EngineError::Rejected {
            reason: "libtorrent worker dropped the reply".to_owned(),
        })?
    }
}

impl TorrentEngine for LibtorrentEngine {
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Add { spec, reply })?;
        Self::recv(response)
    }

    fn restore_torrent(&mut self, id: TorrentId, spec: AddTorrent) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Restore { id, spec, reply })?;
        Self::recv(response)
    }

    fn request_resume_data(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::RequestResume { id, reply })?;
        Self::recv(response)
    }

    fn take_resume_data(&mut self, id: TorrentId) -> Result<Option<Vec<u8>>, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::TakeResume { id, reply })?;
        Self::recv(response)
    }

    fn pause(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Pause { id, reply })?;
        Self::recv(response)
    }

    fn resume(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Resume { id, reply })?;
        Self::recv(response)
    }

    fn remove(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Remove { id, reply })?;
        Self::recv(response)
    }

    fn list(&self) -> Vec<TorrentStatus> {
        let (reply, response) = mpsc::channel();
        if self.send(Command::List { reply }).is_err() {
            return Vec::new();
        }
        response.recv().unwrap_or_default()
    }

    fn status(&self, id: TorrentId) -> Result<TorrentStatus, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Status { id, reply })?;
        Self::recv(response)
    }

    fn files(&mut self, id: TorrentId) -> Result<Vec<crate::TorrentFileInfo>, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Files { id, reply })?;
        Self::recv(response)
    }

    fn peers(&mut self, id: TorrentId) -> Result<Vec<crate::TorrentPeerInfo>, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Peers { id, reply })?;
        Self::recv(response)
    }

    fn trackers(&mut self, id: TorrentId) -> Result<Vec<crate::TorrentTrackerInfo>, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Trackers { id, reply })?;
        Self::recv(response)
    }

    fn settings(&self) -> EngineSettings {
        self.settings
    }

    fn set_settings(&mut self, settings: EngineSettings) {
        let (reply, response) = mpsc::channel();
        if self.send(Command::SetSettings { settings, reply }).is_ok() {
            let _ = response.recv();
            self.settings = settings;
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }
}

impl Drop for LibtorrentEngine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    commands: Receiver<Command>,
    events: broadcast::Sender<EngineEvent>,
    settings: EngineSettings,
) {
    let mut session = new_session(
        settings.download_rate_limit,
        settings.upload_rate_limit,
        settings.seed_mode,
    );
    let mut last_progress = Instant::now();

    loop {
        match commands.recv_timeout(POLL_INTERVAL) {
            Ok(Command::Add { spec, reply }) => {
                let metainfo = match &spec.source {
                    TorrentSource::Metainfo(bytes) => bytes.clone(),
                    TorrentSource::Magnet(_) => {
                        let _ = reply.send(Err(EngineError::Rejected {
                            reason: "magnet URIs are not supported by the libtorrent backend yet"
                                .to_owned(),
                        }));
                        continue;
                    }
                };
                let save_path = spec.save_path.to_string_lossy().into_owned();
                let resume_data = spec.resume_data.as_deref().unwrap_or(&[]);
                let id = session
                    .pin_mut()
                    .add_torrent(&metainfo, &save_path, resume_data);
                if id == 0 {
                    let _ = reply.send(Err(EngineError::Rejected {
                        reason: session.last_error().to_string(),
                    }));
                    continue;
                }
                let status = convert_status(session.status(id));
                let _ = events.send(EngineEvent::Added(status));
                let _ = reply.send(Ok(id));
            }
            Ok(Command::Restore { id, spec, reply }) => {
                let metainfo = match &spec.source {
                    TorrentSource::Metainfo(bytes) => bytes.clone(),
                    TorrentSource::Magnet(_) => {
                        let _ = reply.send(Err(EngineError::Rejected {
                            reason: "magnet URIs are not supported by the libtorrent backend yet"
                                .to_owned(),
                        }));
                        continue;
                    }
                };
                let save_path = spec.save_path.to_string_lossy().into_owned();
                let resume_data = spec.resume_data.as_deref().unwrap_or(&[]);
                if session
                    .pin_mut()
                    .restore_torrent(id, &metainfo, &save_path, resume_data)
                {
                    let status = convert_status(session.status(id));
                    let _ = events.send(EngineEvent::Added(status));
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(EngineError::Rejected {
                        reason: session.last_error().to_string(),
                    }));
                }
            }
            Ok(Command::RequestResume { id, reply }) => {
                let ok = session.pin_mut().request_resume(id);
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::TakeResume { id, reply }) => {
                let data: Vec<u8> = session.pin_mut().take_resume(id).into_iter().collect();
                let _ = reply.send(Ok(if data.is_empty() { None } else { Some(data) }));
            }
            Ok(Command::Pause { id, reply }) => {
                let ok = session.pin_mut().pause(id);
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::Resume { id, reply }) => {
                let ok = session.pin_mut().resume(id);
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::Remove { id, reply }) => {
                let ok = session.pin_mut().remove(id);
                if ok {
                    let _ = events.send(EngineEvent::Removed { id });
                }
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::Files { id, reply }) => {
                let files = session
                    .files(id)
                    .into_iter()
                    .map(|file| crate::TorrentFileInfo {
                        path: file.path.to_string(),
                        size: file.size,
                        progress: file.progress,
                        priority: file.priority,
                    })
                    .collect();
                let _ = reply.send(Ok(files));
            }
            Ok(Command::Peers { id, reply }) => {
                let peers = session
                    .peers(id)
                    .into_iter()
                    .map(|peer| crate::TorrentPeerInfo {
                        endpoint: peer.endpoint.to_string(),
                        client: peer.client.to_string(),
                        progress: peer.progress,
                        download_rate: peer.download_rate,
                        upload_rate: peer.upload_rate,
                    })
                    .collect();
                let _ = reply.send(Ok(peers));
            }
            Ok(Command::Trackers { id, reply }) => {
                let trackers = session
                    .trackers(id)
                    .into_iter()
                    .map(|tracker| crate::TorrentTrackerInfo {
                        url: tracker.url.to_string(),
                        status: tracker.status.to_string(),
                        seeds: tracker.seeds,
                        leechers: tracker.leechers,
                    })
                    .collect();
                let _ = reply.send(Ok(trackers));
            }
            Ok(Command::List { reply }) => {
                let list = session.list().into_iter().map(convert_status).collect();
                let _ = reply.send(list);
            }
            Ok(Command::Status { id, reply }) => {
                let status = session.status(id);
                let result = if status.found {
                    Ok(convert_status(status))
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
            }
            Ok(Command::SetSettings { settings, reply }) => {
                session
                    .pin_mut()
                    .set_settings(settings.download_rate_limit, settings.upload_rate_limit);
                let _ = reply.send(());
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        for event in session.pin_mut().poll_events() {
            if let Some(event) = convert_event(event) {
                let _ = events.send(event);
            }
        }

        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            session.pin_mut().request_updates();
        }
    }
}

fn state_from_code(code: i32, error: String) -> TorrentState {
    match code {
        0 => TorrentState::Queued,
        1 => TorrentState::Checking,
        2 => TorrentState::Downloading,
        3 => TorrentState::Seeding,
        4 => TorrentState::Paused,
        _ => TorrentState::Error(if error.is_empty() {
            "libtorrent error".to_owned()
        } else {
            error
        }),
    }
}

fn convert_status(status: FfiStatus) -> TorrentStatus {
    let error = status.error.to_string();
    TorrentStatus {
        id: status.id,
        name: status.name.to_string(),
        total_bytes: status.total_bytes,
        state: state_from_code(status.state, error),
        progress: status.progress,
        download_rate: status.download_rate,
        upload_rate: status.upload_rate,
        seeds: status.seeds.max(0).unsigned_abs(),
        peers: status.peers.max(0).unsigned_abs(),
        eta_seconds: u32::try_from(status.eta_seconds.max(0)).unwrap_or(u32::MAX),
    }
}

fn convert_event(event: FfiEvent) -> Option<EngineEvent> {
    match event.kind {
        1 => Some(EngineEvent::StateChanged {
            id: event.id,
            state: state_from_code(event.state, String::new()),
        }),
        2 => Some(EngineEvent::Progress {
            id: event.id,
            progress: event.progress,
            download_rate: event.download_rate,
            upload_rate: event.upload_rate,
        }),
        3 => Some(EngineEvent::Removed { id: event.id }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/hello.torrent");

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrent-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn payload() -> Vec<u8> {
        "hello ferrent\n".repeat(1024).into_bytes()
    }

    #[test]
    fn add_pause_resume_remove_roundtrip() {
        let dir = temp_dir("libtorrent-roundtrip");
        let data = payload();
        fs::write(dir.join("payload.bin"), &data).expect("write payload");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let id = engine
            .add_torrent(AddTorrent {
                name: "payload.bin".to_owned(),
                total_bytes: data.len() as u64,
                source: TorrentSource::Metainfo(FIXTURE.to_vec()),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add torrent");

        let status = engine.status(id).expect("status");
        assert_eq!(status.id, id);
        assert_eq!(status.total_bytes, data.len() as u64);
        assert!(engine.list().iter().any(|status| status.id == id));

        engine.pause(id).expect("pause");
        assert_eq!(
            engine.status(id).expect("paused status").state,
            TorrentState::Paused
        );

        engine.resume(id).expect("resume");
        engine.remove(id).expect("remove");
        assert_eq!(engine.status(id), Err(EngineError::NotFound { id }));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_data_restores_without_recheck() {
        let dir = temp_dir("libtorrent-resume");
        let data = payload();
        fs::write(dir.join("payload.bin"), &data).expect("write payload");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let id = engine
            .add_torrent(AddTorrent {
                name: "payload.bin".to_owned(),
                total_bytes: data.len() as u64,
                source: TorrentSource::Metainfo(FIXTURE.to_vec()),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add torrent");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = engine.status(id).expect("status");
            if status.progress >= 0.999 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "torrent did not finish: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        engine.request_resume_data(id).expect("request resume");
        let deadline = Instant::now() + Duration::from_secs(10);
        let resume = loop {
            if let Some(data) = engine.take_resume_data(id).expect("take resume") {
                break data;
            }
            assert!(Instant::now() < deadline, "no resume data");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(!resume.is_empty());
        engine.remove(id).expect("remove original");
        drop(engine);

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        engine
            .restore_torrent(
                id,
                AddTorrent {
                    name: "payload.bin".to_owned(),
                    total_bytes: data.len() as u64,
                    source: TorrentSource::Metainfo(FIXTURE.to_vec()),
                    save_path: dir.clone(),
                    resume_data: Some(resume),
                },
            )
            .expect("restore torrent");
        std::thread::sleep(Duration::from_millis(500));

        let status = engine.status(id).expect("restored status");
        assert!(
            status.progress >= 0.999,
            "restored torrent did not use resume data: {status:?}"
        );
        engine.remove(id).expect("remove restored");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn magnet_source_is_rejected() {
        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let error = engine
            .add_torrent(AddTorrent {
                name: "magnet".to_owned(),
                total_bytes: 0,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:0123456789abcdef".to_owned()),
                save_path: temp_dir("libtorrent-magnet"),
                resume_data: None,
            })
            .expect_err("magnet should be rejected");
        assert!(matches!(error, EngineError::Rejected { .. }));
    }
}
