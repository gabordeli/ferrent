//! `libtorrent` backend. The C++ session lives on a dedicated worker thread;
//! the Rust side talks to it over a command channel and receives deltas on a
//! broadcast channel.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use crate::engine::TorrentEngine;
use crate::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentId, TorrentLimits, TorrentSource,
    TorrentState, TorrentStatus,
};

use self::bridge::ffi::{FfiEvent, FfiStatus, make_torrent_from_path, new_session};

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
    SetLimits {
        id: TorrentId,
        limits: TorrentLimits,
        reply: Sender<Result<(), EngineError>>,
    },
    MoveStorage {
        id: TorrentId,
        path: PathBuf,
        reply: Sender<Result<(), EngineError>>,
    },
    Remove {
        id: TorrentId,
        delete_files: bool,
        reply: Sender<Result<(), EngineError>>,
    },
    Move {
        id: TorrentId,
        up: bool,
        reply: Sender<Result<(), EngineError>>,
    },
    Files {
        id: TorrentId,
        reply: Sender<Result<Vec<crate::TorrentFileInfo>, EngineError>>,
    },
    ForceRecheck {
        id: TorrentId,
        reply: Sender<Result<(), EngineError>>,
    },
    SetSequential {
        id: TorrentId,
        on: bool,
        reply: Sender<Result<(), EngineError>>,
    },
    SetSuperSeeding {
        id: TorrentId,
        on: bool,
        reply: Sender<Result<(), EngineError>>,
    },
    RenameFile {
        id: TorrentId,
        index: usize,
        name: String,
        reply: Sender<Result<(), EngineError>>,
    },
    SetFilePriorities {
        id: TorrentId,
        priorities: Vec<i32>,
        reply: Sender<Result<(), EngineError>>,
    },
    CreateTorrent {
        spec: crate::CreateTorrentSpec,
        reply: Sender<Result<crate::CreatedTorrent, EngineError>>,
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
        let worker_settings = settings.clone();
        let worker = thread::Builder::new()
            .name("ferrent-libtorrent".to_owned())
            .spawn(move || run_worker(command_rx, worker_events, worker_settings))
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

    fn set_torrent_limits(
        &mut self,
        id: TorrentId,
        limits: TorrentLimits,
    ) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::SetLimits { id, limits, reply })?;
        Self::recv(response)
    }

    fn move_storage(&mut self, id: TorrentId, path: PathBuf) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::MoveStorage { id, path, reply })?;
        Self::recv(response)
    }

    fn remove(&mut self, id: TorrentId, delete_files: bool) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Remove {
            id,
            delete_files,
            reply,
        })?;
        Self::recv(response)
    }

    fn move_in_queue(
        &mut self,
        id: TorrentId,
        direction: crate::QueueDirection,
    ) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::Move {
            id,
            up: direction == crate::QueueDirection::Up,
            reply,
        })?;
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

    fn force_recheck(&mut self, id: TorrentId) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::ForceRecheck { id, reply })?;
        Self::recv(response)
    }

    fn set_sequential(&mut self, id: TorrentId, on: bool) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::SetSequential { id, on, reply })?;
        Self::recv(response)
    }

    fn set_super_seeding(&mut self, id: TorrentId, on: bool) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::SetSuperSeeding { id, on, reply })?;
        Self::recv(response)
    }

    fn rename_file(&mut self, id: TorrentId, index: usize, name: &str) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::RenameFile {
            id,
            index,
            name: name.to_owned(),
            reply,
        })?;
        Self::recv(response)
    }

    fn set_file_priorities(
        &mut self,
        id: TorrentId,
        priorities: &[i32],
    ) -> Result<(), EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::SetFilePriorities {
            id,
            priorities: priorities.to_vec(),
            reply,
        })?;
        Self::recv(response)
    }

    fn create_torrent(
        &mut self,
        spec: crate::CreateTorrentSpec,
    ) -> Result<crate::CreatedTorrent, EngineError> {
        let (reply, response) = mpsc::channel();
        self.send(Command::CreateTorrent { spec, reply })?;
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
        self.settings.clone()
    }

    fn set_settings(&mut self, settings: EngineSettings) {
        let (reply, response) = mpsc::channel();
        if self
            .send(Command::SetSettings {
                settings: settings.clone(),
                reply,
            })
            .is_ok()
        {
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
        settings.active_downloads,
        settings.active_seeds,
        settings.active_limit,
        &settings.listen_interfaces,
        &settings.outgoing_interfaces,
    );
    session
        .pin_mut()
        .set_default_trackers(&settings.default_trackers.join("\n"));
    session.pin_mut().set_network_settings(
        &settings.proxy_type,
        &settings.proxy_host,
        settings.proxy_port,
        &settings.proxy_username,
        &settings.proxy_password,
        settings.proxy_peer_connections,
        &settings.encryption,
        settings.upnp,
        settings.natpmp,
    );
    let mut last_progress = Instant::now();
    let mut finished = std::collections::BTreeSet::new();

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
                let result = session
                    .pin_mut()
                    .add_torrent(&metainfo, &save_path, resume_data);
                if result.duplicate {
                    let _ = reply.send(Err(EngineError::AlreadyExists { id: result.id }));
                    continue;
                }
                if !result.ok {
                    let _ = reply.send(Err(EngineError::Rejected {
                        reason: result.error.to_string(),
                    }));
                    continue;
                }
                let id = result.id;
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
                    if status.progress >= 0.999 {
                        finished.insert(id);
                    }
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
            Ok(Command::SetLimits { id, limits, reply }) => {
                let ok = session.pin_mut().set_torrent_limits(
                    id,
                    limits.download_rate_limit,
                    limits.upload_rate_limit,
                    limits.connections_limit,
                    limits.upload_slots_limit,
                );
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::MoveStorage { id, path, reply }) => {
                let path = path.to_string_lossy().into_owned();
                let ok = session.pin_mut().move_storage(id, &path);
                if ok {
                    let _ = events.send(EngineEvent::StateChanged {
                        id,
                        state: TorrentState::Moving,
                    });
                }
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::Remove {
                id,
                delete_files,
                reply,
            }) => {
                let ok = session.pin_mut().remove(id, delete_files);
                if ok {
                    let _ = events.send(EngineEvent::Removed { id });
                }
                let _ = reply.send(if ok {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                });
            }
            Ok(Command::Move { id, up, reply }) => {
                let ok = session.pin_mut().move_in_queue(id, up);
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
            Ok(Command::ForceRecheck { id, reply }) => {
                let result = if session.pin_mut().force_recheck(id) {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
            }
            Ok(Command::SetSequential { id, on, reply }) => {
                let result = if session.pin_mut().set_sequential(id, on) {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
            }
            Ok(Command::SetSuperSeeding { id, on, reply }) => {
                let result = if session.pin_mut().set_super_seeding(id, on) {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
            }
            Ok(Command::RenameFile {
                id,
                index,
                name,
                reply,
            }) => {
                let result = if session.pin_mut().rename_file(id, index as i32, &name) {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
            }
            Ok(Command::SetFilePriorities {
                id,
                priorities,
                reply,
            }) => {
                let result = if session.pin_mut().set_file_priorities(id, &priorities) {
                    Ok(())
                } else {
                    Err(EngineError::NotFound { id })
                };
                let _ = reply.send(result);
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
                        message: tracker.message.to_string(),
                        next_announce: tracker.next_announce,
                        fails: tracker.fails,
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
                session.pin_mut().set_settings(
                    settings.download_rate_limit,
                    settings.upload_rate_limit,
                    settings.active_downloads,
                    settings.active_seeds,
                    settings.active_limit,
                    settings.seed_mode,
                );
                session
                    .pin_mut()
                    .set_default_trackers(&settings.default_trackers.join("\n"));
                session.pin_mut().set_network_settings(
                    &settings.proxy_type,
                    &settings.proxy_host,
                    settings.proxy_port,
                    &settings.proxy_username,
                    &settings.proxy_password,
                    settings.proxy_peer_connections,
                    &settings.encryption,
                    settings.upnp,
                    settings.natpmp,
                );
                let _ = reply.send(());
            }
            Ok(Command::CreateTorrent { spec, reply }) => {
                let trackers = spec.trackers.join("\n");
                let created = make_torrent_from_path(
                    &spec.path.to_string_lossy(),
                    spec.piece_length,
                    &trackers,
                );
                let result = if created.ok {
                    Ok(crate::CreatedTorrent {
                        metainfo: created.metainfo,
                        infohash: created.infohash.to_string(),
                    })
                } else {
                    Err(EngineError::Rejected {
                        reason: created.error.to_string(),
                    })
                };
                let _ = reply.send(result);
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        for event in session.pin_mut().poll_events() {
            if let Some(event) = convert_event(event) {
                if let EngineEvent::Finished { id } = &event {
                    if !finished.insert(*id) {
                        continue;
                    }
                }
                if let EngineEvent::Removed { id } = &event {
                    finished.remove(id);
                }
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
        6 => TorrentState::Moving,
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
        infohash: status.infohash.to_string(),
        name: status.name.to_string(),
        save_path: PathBuf::from(status.save_path.to_string()),
        total_bytes: status.total_bytes,
        state: state_from_code(status.state, error),
        progress: status.progress,
        download_rate: status.download_rate,
        upload_rate: status.upload_rate,
        uploaded_bytes: status.uploaded_bytes,
        downloaded_bytes: status.downloaded_bytes,
        seeds: status.seeds.max(0).unsigned_abs(),
        peers: status.peers.max(0).unsigned_abs(),
        eta_seconds: u32::try_from(status.eta_seconds.max(0)).unwrap_or(u32::MAX),
        queue_position: status.queue_position,
        sequential: status.sequential,
        super_seeding: status.super_seeding,
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
            queue_position: event.queue_position,
            seeds: event.seeds,
            peers: event.peers,
        }),
        3 => Some(EngineEvent::Removed { id: event.id }),
        4 => Some(EngineEvent::Finished { id: event.id }),
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

    /// A directory on a different filesystem when `/dev/shm` exists.
    fn cross_fs_dir(name: &str) -> PathBuf {
        let base = if std::path::Path::new("/dev/shm").is_dir() {
            PathBuf::from("/dev/shm")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("ferrent-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create cross-fs dir");
        dir
    }

    fn payload() -> Vec<u8> {
        "hello ferrent\n".repeat(1024).into_bytes()
    }

    #[test]
    fn file_priorities_round_trip_through_the_bridge() {
        let dir = temp_dir("libtorrent-priorities");
        let data = payload();
        fs::write(dir.join("payload.bin"), &data).expect("write payload");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let spec = AddTorrent {
            name: "payload.bin".to_owned(),
            total_bytes: data.len() as u64,
            source: TorrentSource::Metainfo(FIXTURE.to_vec()),
            save_path: dir.clone(),
            resume_data: None,
        };
        let id = engine.add_torrent(spec).expect("add torrent");
        assert_eq!(engine.files(id).expect("files").len(), 1);

        engine.set_file_priorities(id, &[0]).expect("skip");
        assert_eq!(
            wait_for_priority(&mut engine, id, 0),
            0,
            "the skipped priority reaches libtorrent"
        );

        engine.set_file_priorities(id, &[7]).expect("high");
        assert_eq!(wait_for_priority(&mut engine, id, 7), 7, "high priority");

        assert!(
            engine.set_file_priorities(999, &[4]).is_err(),
            "an unknown torrent id is an error, not a silent no-op"
        );
        engine.remove(id, true).expect("remove");
    }

    #[test]
    fn create_torrent_from_a_directory_round_trips_through_add() {
        let dir = temp_dir("libtorrent-create");
        let source = dir.join("album");
        fs::create_dir_all(&source).expect("source dir");
        fs::write(source.join("one.bin"), b"one".repeat(1024)).expect("write one");
        fs::write(source.join("two.bin"), b"two".repeat(2048)).expect("write two");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let created = engine
            .create_torrent(crate::CreateTorrentSpec {
                path: source.clone(),
                piece_length: 16 * 1024,
                trackers: vec!["udp://tracker.example:6969".to_owned()],
            })
            .expect("create torrent");
        assert!(!created.metainfo.is_empty());
        assert_eq!(created.infohash.len(), 40);

        let id = engine
            .add_torrent(AddTorrent {
                name: "album".to_owned(),
                total_bytes: 0,
                source: TorrentSource::Metainfo(created.metainfo.clone()),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add the created torrent");

        let status = engine.status(id).expect("status");
        assert_eq!(status.infohash, created.infohash, "the info hash matches");
        let files = engine.files(id).expect("files");
        assert_eq!(files.len(), 2, "both files are in the metainfo");
        assert!(files.iter().any(|file| file.path.ends_with("one.bin")));

        // An empty directory is a user error, not a panic.
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).expect("empty dir");
        let error = engine
            .create_torrent(crate::CreateTorrentSpec {
                path: empty,
                piece_length: 0,
                trackers: Vec::new(),
            })
            .expect_err("empty directory");
        assert!(matches!(error, EngineError::Rejected { .. }));

        engine.remove(id, true).expect("remove");
    }

    #[test]
    fn create_torrent_hashes_several_pieces() {
        let dir = temp_dir("libtorrent-create-multi");
        let source = dir.join("album");
        fs::create_dir_all(&source).expect("source dir");
        fs::write(source.join("one.bin"), vec![1_u8; 1_000_000]).expect("write one");
        fs::write(source.join("two.bin"), vec![2_u8; 250_000]).expect("write two");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let created = engine
            .create_torrent(crate::CreateTorrentSpec {
                path: source.clone(),
                piece_length: 0,
                trackers: Vec::new(),
            })
            .expect("create torrent");
        // Five pieces of 256 KiB: the metainfo carries 5 x 20 bytes of hashes.
        assert!(created.metainfo.len() > 200);
        assert_eq!(created.infohash.len(), 40);

        let id = engine
            .add_torrent(AddTorrent {
                name: "album".to_owned(),
                total_bytes: 0,
                source: TorrentSource::Metainfo(created.metainfo),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add the created torrent");
        assert_eq!(engine.status(id).expect("status").total_bytes, 1_250_000);
        engine.remove(id, true).expect("remove");
    }

    /// libtorrent applies file priorities asynchronously, so poll briefly.
    fn wait_for_priority(engine: &mut LibtorrentEngine, id: TorrentId, wanted: i32) -> i32 {
        for _ in 0..100 {
            let priority = engine.files(id).expect("files")[0].priority;
            if priority == wanted {
                return priority;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        engine.files(id).expect("files")[0].priority
    }

    #[test]
    fn add_pause_resume_remove_roundtrip() {
        let dir = temp_dir("libtorrent-roundtrip");
        let data = payload();
        fs::write(dir.join("payload.bin"), &data).expect("write payload");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let spec = AddTorrent {
            name: "payload.bin".to_owned(),
            total_bytes: data.len() as u64,
            source: TorrentSource::Metainfo(FIXTURE.to_vec()),
            save_path: dir.clone(),
            resume_data: None,
        };
        let id = engine.add_torrent(spec.clone()).expect("add torrent");

        let status = engine.status(id).expect("status");
        assert_eq!(status.id, id);
        assert_eq!(status.total_bytes, data.len() as u64);
        assert_eq!(status.infohash.len(), 40);
        assert!(engine.list().iter().any(|status| status.id == id));

        assert_eq!(
            engine.add_torrent(spec),
            Err(EngineError::AlreadyExists { id })
        );
        engine
            .set_torrent_limits(
                id,
                TorrentLimits {
                    download_rate_limit: 1_048_576,
                    upload_rate_limit: 524_288,
                    connections_limit: 20,
                    upload_slots_limit: 4,
                },
            )
            .expect("set limits");

        engine.pause(id).expect("pause");
        assert_eq!(
            engine.status(id).expect("paused status").state,
            TorrentState::Paused
        );

        engine.resume(id).expect("resume");
        engine.remove(id, false).expect("remove");
        assert_eq!(engine.status(id), Err(EngineError::NotFound { id }));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_storage_reports_moving_and_moves_the_payload() {
        let dir = temp_dir("libtorrent-move-src");
        let target = cross_fs_dir("libtorrent-move-dst");
        let data = payload();
        fs::write(dir.join("payload.bin"), &data).expect("write payload");

        let mut engine = LibtorrentEngine::new(EngineSettings::default());
        let mut events = engine.subscribe();
        let id = engine
            .add_torrent(AddTorrent {
                name: "payload.bin".to_owned(),
                total_bytes: data.len() as u64,
                source: TorrentSource::Metainfo(FIXTURE.to_vec()),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add torrent");

        engine
            .move_storage(id, target.clone())
            .expect("move storage");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_moving = false;
        let status = loop {
            while let Ok(event) = events.try_recv() {
                if let EngineEvent::StateChanged { state, .. } = event {
                    saw_moving = saw_moving || state == TorrentState::Moving;
                }
            }
            let status = engine.status(id).expect("status");
            if status.save_path == target && status.state != TorrentState::Moving {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "storage did not move: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(saw_moving, "no Moving state was emitted");
        assert_eq!(status.save_path, target);
        assert!(
            target.join("payload.bin").exists(),
            "payload was not moved to the target"
        );

        engine.remove(id, false).expect("remove");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&target);
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
        engine.remove(id, false).expect("remove original");
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
        engine.remove(id, false).expect("remove restored");
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
