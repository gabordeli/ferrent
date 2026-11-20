//! gRPC server: TLS transport, bearer-token auth, and the `Ferrent` service.

pub mod tls;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ferrent_engine::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, TorrentEngine, TorrentId, TorrentLimits,
    TorrentSource, TorrentState as EngineState, TorrentStatus,
};
use ferrent_protocol::v1::ferrent_server::{Ferrent, FerrentServer};
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ferrent_protocol::v1::{
    AddTorrentRequest, AddTorrentResponse, AutoAddDirStatus, ConfigChanged, CreateLabelRequest,
    DeleteLabelRequest, DeleteLabelResponse, Disk as ProtoDisk, DiskZone as ProtoDiskZone,
    GetAutoAddStatusRequest, GetAutoAddStatusResponse, GetConfigRequest, GetConfigResponse,
    GetDisksRequest, GetDisksResponse, GetHookStatusRequest, GetHookStatusResponse,
    GetNotificationStatusRequest, GetNotificationStatusResponse, GetStatusRequest,
    GetStatusResponse, GetTorrentFilesRequest, GetTorrentFilesResponse, GetTorrentPeersRequest,
    GetTorrentPeersResponse, GetTorrentRequest, GetTorrentResponse, GetTorrentTrackersRequest,
    GetTorrentTrackersResponse, HookRun as ProtoHookRun, Label as ProtoLabel, LabelEntry,
    LabelResponse, ListLabelsRequest, ListLabelsResponse, ListTorrentsRequest,
    ListTorrentsResponse, MoveTorrentRequest, MoveTorrentResponse, PauseTorrentRequest,
    PauseTorrentResponse, Progress, QueueDirection as ProtoQueueDirection, RemoveTorrentRequest,
    RemoveTorrentResponse, Removed, ResumeTorrentRequest, ResumeTorrentResponse,
    ScanAutoAddRequest, ScanAutoAddResponse, SendTestNotificationRequest,
    SendTestNotificationResponse, SetConfigRequest, SetConfigResponse, SetTorrentLabelRequest,
    SetTorrentLabelResponse, SetTorrentLimitsRequest, SetTorrentLimitsResponse, SortField,
    StateChanged, Torrent, TorrentFile, TorrentFinished, TorrentPeer, TorrentState, TorrentTracker,
    UpdateLabelRequest, WatchRequest, WatchResponse,
};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{BroadcastStream, TcpListenerStream};
use tonic::metadata::MetadataMap;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};

use crate::config::{ConfigError, ConfigStore};
use crate::control::{PauseReason, TorrentControl};
use crate::labels::{self, Label as DomainLabel, LabelState};
use crate::persist::{self, PersistedTorrent};
use crate::state::{StateMirror, TorrentQuery, TorrentSort, run_mirror, state_code};
use crate::store::StateStore;

pub use tls::TlsMaterial;

/// Bearer-token authentication.
#[derive(Clone)]
pub struct Auth {
    token: String,
}

impl Auth {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    /// Validates the `authorization: Bearer <token>` metadata.
    pub fn check(&self, metadata: &MetadataMap) -> Result<(), Status> {
        let provided = metadata
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let expected = format!("Bearer {}", self.token);
        if bool::from(provided.as_bytes().ct_eq(expected.as_bytes())) {
            Ok(())
        } else {
            Err(Status::unauthenticated("missing or invalid bearer token"))
        }
    }
}

/// Snapshot of the mirror used by `GetStatus`.
#[derive(Debug, Default)]
pub struct AggregateStatus {
    pub torrent_count: u32,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub free_space: u64,
    pub downloading: u32,
    pub seeding: u32,
    pub queued: u32,
    pub paused: u32,
    pub error: u32,
    pub checking: u32,
    pub moving: u32,
    pub active: u32,
}

/// Shared daemon state used by the RPC service.
pub struct AppState {
    pub engine: Arc<Mutex<Box<dyn TorrentEngine>>>,
    pub(crate) mirror: Arc<StateMirror>,
    pub(crate) watch: broadcast::Sender<EngineEvent>,
    pub config: ConfigStore,
    pub(crate) config_watch: broadcast::Sender<u64>,
    pub control: Arc<Mutex<TorrentControl>>,
    pub labels: Arc<Mutex<LabelState>>,
    pub store: Arc<Mutex<Box<dyn StateStore>>>,
    pub(crate) auth: Auth,
    pub(crate) data_dir: PathBuf,
    /// AutoAdd scanner counters, shared with `GetAutoAddStatus`.
    pub(crate) autoadd: Mutex<crate::autoadd::AutoAddStatus>,
    /// Scheduler runtime state, shared with `GetStatus`.
    pub(crate) scheduler: Mutex<crate::scheduler::SchedulerState>,
    /// Completion moves waiting for space or a target folder.
    pub(crate) moves: Mutex<BTreeMap<TorrentId, crate::diskguard::moves::PendingMove>>,
    /// Last disk guard payload, kept across a failed scan.
    pub(crate) disks: Mutex<Option<crate::diskguard::StorageReport>>,
    /// Disk guard threshold state per filesystem.
    pub(crate) disk_levels: Mutex<BTreeMap<PathBuf, crate::diskguard::guard::Level>>,
}

impl AppState {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: Box<dyn TorrentEngine>,
        mirror: Arc<StateMirror>,
        watch: broadcast::Sender<EngineEvent>,
        store: Box<dyn StateStore>,
        config: ConfigStore,
        control: TorrentControl,
        labels: LabelState,
        token: impl Into<String>,
        data_dir: PathBuf,
    ) -> Self {
        let (config_watch, _) = broadcast::channel(16);
        Self {
            engine: Arc::new(Mutex::new(engine)),
            mirror,
            watch,
            config,
            config_watch,
            control: Arc::new(Mutex::new(control)),
            labels: Arc::new(Mutex::new(labels)),
            store: Arc::new(Mutex::new(store)),
            auth: Auth::new(token),
            data_dir,
            autoadd: Mutex::new(crate::autoadd::AutoAddStatus::default()),
            scheduler: Mutex::new(crate::scheduler::SchedulerState::default()),
            moves: Mutex::new(BTreeMap::new()),
            disks: Mutex::new(None),
            disk_levels: Mutex::new(BTreeMap::new()),
        }
    }

    /// Applies one pause reason to many torrents.
    ///
    /// The scheduler and the disk guard use this; the engine only sees the
    /// torrents whose effective state changed.
    pub async fn set_pause_reason(
        &self,
        ids: &[TorrentId],
        reason: PauseReason,
        active: bool,
    ) -> Result<usize, Status> {
        let changed = {
            let mut control = self.control.lock().expect("pause control mutex poisoned");
            control.set_many(ids, reason, active)
        };
        if changed.is_empty() {
            return Ok(0);
        }
        let engine = Arc::clone(&self.engine);
        tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            if active {
                engine.pause_many(&changed)
            } else {
                engine.resume_many(&changed)
            }
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))
    }

    #[must_use]
    pub fn aggregate_status(&self) -> AggregateStatus {
        let torrents = self.mirror.query(&TorrentQuery::default());
        let mut status = AggregateStatus {
            torrent_count: torrents.len() as u32,
            free_space: rustix::fs::statvfs(&self.data_dir)
                .map(|stat| stat.f_bavail.saturating_mul(stat.f_frsize))
                .unwrap_or(0),
            ..AggregateStatus::default()
        };
        for torrent in &torrents {
            status.download_rate = status.download_rate.saturating_add(torrent.download_rate);
            status.upload_rate = status.upload_rate.saturating_add(torrent.upload_rate);
            if torrent.download_rate > 0 || torrent.upload_rate > 0 {
                status.active = status.active.saturating_add(1);
            }
            match state_code(&torrent.state) {
                1 => status.queued += 1,
                2 => status.checking += 1,
                3 => status.downloading += 1,
                4 => status.seeding += 1,
                5 => status.paused += 1,
                6 => status.error += 1,
                7 => status.moving += 1,
                _ => {}
            }
        }
        status
    }

    #[must_use]
    pub fn engine_handle(&self) -> Arc<Mutex<Box<dyn TorrentEngine>>> {
        Arc::clone(&self.engine)
    }

    #[must_use]
    pub fn store_handle(&self) -> Arc<Mutex<Box<dyn StateStore>>> {
        Arc::clone(&self.store)
    }

    #[must_use]
    pub fn mirror_handle(&self) -> Arc<StateMirror> {
        Arc::clone(&self.mirror)
    }
}

/// `Ferrent` service implementation.
#[derive(Clone)]
pub struct FerrentService {
    state: Arc<AppState>,
}

impl FerrentService {
    #[must_use]
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    async fn with_engine<T, F>(&self, operation: F) -> Result<T, Status>
    where
        T: Send + 'static,
        F: FnOnce(&mut dyn TorrentEngine) -> Result<T, EngineError> + Send + 'static,
    {
        let engine = Arc::clone(&self.state.engine);
        tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            operation(&mut **engine)
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?
        .map_err(engine_status)
    }

    async fn persist<F>(&self, operation: F) -> Result<(), Status>
    where
        F: FnOnce(&mut dyn StateStore) -> Result<(), crate::store::StoreError> + Send + 'static,
    {
        let store = Arc::clone(&self.state.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            operation(&mut **store)
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?
        .map_err(|error| Status::internal(format!("state store: {error}")))
    }

    async fn apply_limits(&self, id: TorrentId) -> Result<(), Status> {
        let limits = {
            let labels = self.state.labels.lock().expect("label state poisoned");
            labels.limits(id)
        };
        self.with_engine(move |engine| engine.set_torrent_limits(id, limits))
            .await
    }

    async fn apply_limits_many(&self, ids: &[TorrentId]) -> Result<(), Status> {
        if ids.is_empty() {
            return Ok(());
        }
        let limits: Vec<(TorrentId, TorrentLimits)> = {
            let labels = self.state.labels.lock().expect("label state poisoned");
            ids.iter().map(|id| (*id, labels.limits(*id))).collect()
        };
        let engine = Arc::clone(&self.state.engine);
        tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            for (id, limits) in limits {
                let _ = engine.set_torrent_limits(id, limits);
            }
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))
    }

    async fn apply_label_limits(&self, name: &str) -> Result<(), Status> {
        let ids: Vec<TorrentId> = {
            let labels = self.state.labels.lock().expect("label state poisoned");
            labels
                .assignments()
                .into_iter()
                .filter(|(_, label)| label == name)
                .map(|(id, _)| id)
                .collect()
        };
        self.apply_limits_many(&ids).await
    }

    async fn match_and_assign(
        &self,
        id: TorrentId,
        name: &str,
        trackers: &[String],
    ) -> Result<Option<String>, Status> {
        let matched = {
            let labels = self.state.labels.lock().expect("label state poisoned");
            labels.match_label(name, trackers)
        };
        let Some(label) = matched else {
            return Ok(None);
        };
        {
            let mut labels = self.state.labels.lock().expect("label state poisoned");
            labels
                .assign(id, Some(label.clone()))
                .map_err(Status::invalid_argument)?;
        }
        let persisted = label.clone();
        self.persist(move |store| persist::save_torrent_label(store, id, Some(persisted.as_str())))
            .await?;
        self.apply_limits(id).await?;
        Ok(Some(label))
    }

    /// Assigns an explicit label (already normalised) and its limits.
    async fn assign_label(&self, id: TorrentId, label: &str) -> Result<(), Status> {
        {
            let mut labels = self.state.labels.lock().expect("label state poisoned");
            labels
                .assign(id, Some(label.to_owned()))
                .map_err(Status::invalid_argument)?;
        }
        let persisted = label.to_owned();
        self.persist(move |store| persist::save_torrent_label(store, id, Some(persisted.as_str())))
            .await?;
        self.apply_limits(id).await
    }

    async fn apply_auto_labels(&self) -> Result<usize, Status> {
        let (has_matchers, assigned) = {
            let labels = self.state.labels.lock().expect("label state poisoned");
            (labels.has_matchers(), labels.assignments())
        };
        if !has_matchers {
            return Ok(0);
        }

        let candidates: Vec<(TorrentId, String)> = self
            .state
            .mirror
            .query(&TorrentQuery::default())
            .into_iter()
            .filter(|status| !assigned.contains_key(&status.id))
            .map(|status| (status.id, status.name))
            .collect();
        if candidates.is_empty() {
            return Ok(0);
        }

        let ids: Vec<TorrentId> = candidates.iter().map(|(id, _)| *id).collect();
        let engine = Arc::clone(&self.state.engine);
        let trackers: Vec<(TorrentId, Vec<String>)> = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            ids.into_iter()
                .map(|id| {
                    let urls = engine
                        .trackers(id)
                        .map(|items| items.into_iter().map(|tracker| tracker.url).collect())
                        .unwrap_or_default();
                    (id, urls)
                })
                .collect()
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        let tracker_map: BTreeMap<TorrentId, Vec<String>> = trackers.into_iter().collect();

        let mut matched = Vec::new();
        {
            let mut labels = self.state.labels.lock().expect("label state poisoned");
            for (id, name) in candidates {
                let urls = tracker_map.get(&id).cloned().unwrap_or_default();
                if let Some(label) = labels.match_label(&name, &urls) {
                    labels
                        .assign(id, Some(label.clone()))
                        .expect("matched label exists");
                    matched.push((id, label));
                }
            }
        }
        for (id, label) in &matched {
            let persisted = label.clone();
            let id = *id;
            self.persist(move |store| {
                persist::save_torrent_label(store, id, Some(persisted.as_str()))
            })
            .await?;
        }
        let ids: Vec<TorrentId> = matched.iter().map(|(id, _)| *id).collect();
        self.apply_limits_many(&ids).await?;
        Ok(matched.len())
    }
}

#[tonic::async_trait]
impl Ferrent for FerrentService {
    async fn add_torrent(
        &self,
        request: Request<AddTorrentRequest>,
    ) -> Result<Response<AddTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let label = crate::labels::Label::normalize_name(&request.label);
        if !label.is_empty()
            && self
                .state
                .labels
                .lock()
                .expect("label state poisoned")
                .get(&label)
                .is_none()
        {
            return Err(Status::invalid_argument(format!(
                "label {label:?} is not defined"
            )));
        }
        let config = self
            .state
            .config
            .resolved()
            .map_err(|error| Status::internal(error.to_string()))?;
        let save_path = crate::diskguard::paths::download_path(&config, &request.save_path, &label)
            .map_err(Status::failed_precondition)?;
        // The client knows the size only for magnets; a metainfo add gets it
        // from the info dictionary, which the preallocate gate needs.
        let total_bytes = if request.total_bytes > 0 {
            request.total_bytes
        } else {
            crate::autoadd::Summary::parse(&request.metainfo)
                .map_or(0, |summary| summary.total_bytes)
        };
        let low = match crate::diskguard::paths::space_gate(&config, &save_path, total_bytes) {
            crate::diskguard::paths::SpaceGate::Ok => None,
            crate::diskguard::paths::SpaceGate::Low(message) => Some(message),
            crate::diskguard::paths::SpaceGate::Reject(message) => {
                return Err(Status::failed_precondition(message));
            }
        };
        let source = if request.metainfo.is_empty() && !request.magnet.is_empty() {
            TorrentSource::Magnet(request.magnet.clone())
        } else {
            TorrentSource::Metainfo(request.metainfo.clone())
        };
        let spec = AddTorrent {
            name: request.name,
            total_bytes,
            source,
            save_path,
            resume_data: None,
        };
        let name = spec.name.clone();
        let match_name = name.clone();
        let save_path = spec.save_path.clone();
        let metainfo = match &spec.source {
            TorrentSource::Metainfo(bytes) => bytes.clone(),
            TorrentSource::Magnet(_) => Vec::new(),
        };
        let engine = Arc::clone(&self.state.engine);
        let result = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            match engine.add_torrent(spec) {
                Ok(id) => {
                    let trackers = engine
                        .trackers(id)
                        .map(|items| items.into_iter().map(|tracker| tracker.url).collect())
                        .unwrap_or_default();
                    let infohash = engine
                        .status(id)
                        .map(|status| status.infohash)
                        .unwrap_or_default();
                    Ok((id, trackers, infohash))
                }
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        let (id, trackers, infohash, duplicate) = match result {
            Ok((id, trackers, infohash)) => (id, trackers, infohash, false),
            Err(EngineError::AlreadyExists { id }) => (id, Vec::new(), String::new(), true),
            Err(error) => return Err(engine_status(error)),
        };
        if !duplicate {
            let persisted = PersistedTorrent {
                id,
                name: name.clone(),
                save_path: save_path.clone(),
                metainfo,
                resume_data: None,
                paused_by_user: false,
                label: None,
            };
            self.persist(move |store| persist::save_torrent(store, &persisted))
                .await?;
            if label.is_empty() {
                self.match_and_assign(id, &match_name, &trackers).await?;
            } else {
                self.assign_label(id, &label).await?;
            }
            if let Some(message) = low {
                let event =
                    crate::events::NewEvent::new(crate::events::EventKind::Error, id, name.clone())
                        .with_error(Some(message));
                if let Err(error) = self.state.publish_event(event).await {
                    eprintln!("hook journal: {error}");
                }
            }
            let event = crate::hooks::added_event(id, &name, &save_path, &infohash);
            if let Err(error) = self.state.publish_event(event).await {
                eprintln!("hook journal: {error}");
            }
        }
        Ok(Response::new(AddTorrentResponse { id, duplicate }))
    }

    async fn pause_torrent(
        &self,
        request: Request<PauseTorrentRequest>,
    ) -> Result<Response<PauseTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let (changed, paused) = {
            let mut control = self
                .state
                .control
                .lock()
                .expect("pause control mutex poisoned");
            let changed = control.set_user(id, true);
            (changed, control.is_paused(id))
        };
        if changed && paused {
            if let Err(error) = self.with_engine(move |engine| engine.pause(id)).await {
                self.state
                    .control
                    .lock()
                    .expect("pause control mutex poisoned")
                    .set_user(id, false);
                return Err(error);
            }
        }
        self.persist(move |store| persist::save_paused(store, id, true))
            .await?;
        Ok(Response::new(PauseTorrentResponse {}))
    }

    async fn resume_torrent(
        &self,
        request: Request<ResumeTorrentRequest>,
    ) -> Result<Response<ResumeTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let (changed, paused) = {
            let mut control = self
                .state
                .control
                .lock()
                .expect("pause control mutex poisoned");
            let changed = control.set_user(id, false);
            (changed, control.is_paused(id))
        };
        if changed && !paused {
            self.with_engine(move |engine| engine.resume(id)).await?;
        }
        self.persist(move |store| persist::save_paused(store, id, false))
            .await?;
        Ok(Response::new(ResumeTorrentResponse {}))
    }

    async fn remove_torrent(
        &self,
        request: Request<RemoveTorrentRequest>,
    ) -> Result<Response<RemoveTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let id = request.id;
        let delete_files = request.delete_files;
        if let Some(status) = self.state.mirror.get(id) {
            let event = crate::events::NewEvent::new(
                crate::events::EventKind::Removed,
                id,
                status.name.clone(),
            )
            .with_path(status.save_path.display().to_string())
            .with_infohash(status.infohash.clone());
            if let Err(error) = self.state.publish_event(event).await {
                eprintln!("hook journal: {error}");
            }
        }
        self.with_engine(move |engine| engine.remove(id, delete_files))
            .await?;
        self.state
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .clear(id);
        self.state
            .labels
            .lock()
            .expect("label state poisoned")
            .remove_torrent(id);
        self.persist(move |store| persist::remove_torrent(store, id))
            .await?;
        Ok(Response::new(RemoveTorrentResponse {}))
    }

    async fn move_torrent(
        &self,
        request: Request<MoveTorrentRequest>,
    ) -> Result<Response<MoveTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let id = request.id;
        let up = direction_from_proto(request.direction) == ferrent_engine::QueueDirection::Up;
        if !self.state.mirror.move_in_queue(id, up) {
            return Err(Status::not_found(format!("torrent {id}")));
        }
        Ok(Response::new(MoveTorrentResponse {}))
    }

    async fn list_labels(
        &self,
        request: Request<ListLabelsRequest>,
    ) -> Result<Response<ListLabelsResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let labels = {
            let state = self.state.labels.lock().expect("label state poisoned");
            let counts = state.counts();
            state
                .list()
                .into_iter()
                .map(|label| LabelEntry {
                    torrent_count: counts.get(&label.name).copied().unwrap_or(0),
                    label: Some(label_to_proto(&label)),
                })
                .collect()
        };
        Ok(Response::new(ListLabelsResponse { labels }))
    }

    async fn create_label(
        &self,
        request: Request<CreateLabelRequest>,
    ) -> Result<Response<LabelResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let label = label_from_proto(request.into_inner().label.unwrap_or_default())?;
        {
            let mut state = self.state.labels.lock().expect("label state poisoned");
            state
                .upsert(label.clone())
                .map_err(Status::invalid_argument)?;
        }
        let persisted = label.clone();
        self.persist(move |store| labels::save_label(store, &persisted))
            .await?;
        self.apply_auto_labels().await?;
        self.apply_label_limits(&label.name).await?;
        Ok(Response::new(LabelResponse {
            label: Some(label_to_proto(&label)),
        }))
    }

    async fn update_label(
        &self,
        request: Request<UpdateLabelRequest>,
    ) -> Result<Response<LabelResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let label = label_from_proto(request.into_inner().label.unwrap_or_default())?;
        {
            let mut state = self.state.labels.lock().expect("label state poisoned");
            state
                .upsert(label.clone())
                .map_err(Status::invalid_argument)?;
        }
        let persisted = label.clone();
        self.persist(move |store| labels::save_label(store, &persisted))
            .await?;
        self.apply_auto_labels().await?;
        self.apply_label_limits(&label.name).await?;
        Ok(Response::new(LabelResponse {
            label: Some(label_to_proto(&label)),
        }))
    }

    async fn delete_label(
        &self,
        request: Request<DeleteLabelRequest>,
    ) -> Result<Response<DeleteLabelResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let name = DomainLabel::normalize_name(&request.into_inner().name);
        let affected = {
            let mut state = self.state.labels.lock().expect("label state poisoned");
            state.remove(&name)
        };
        let persisted_name = name.clone();
        self.persist(move |store| labels::delete_label(store, &persisted_name))
            .await?;
        for id in &affected {
            let id = *id;
            self.persist(move |store| persist::save_torrent_label(store, id, None))
                .await?;
        }
        self.apply_limits_many(&affected).await?;
        Ok(Response::new(DeleteLabelResponse {
            removed: true,
            cleared_assignments: affected.len() as u32,
        }))
    }

    async fn set_torrent_label(
        &self,
        request: Request<SetTorrentLabelRequest>,
    ) -> Result<Response<SetTorrentLabelResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let id = request.id;
        let name = DomainLabel::normalize_name(&request.label);
        let label = (!name.is_empty()).then_some(name);
        {
            let mut state = self.state.labels.lock().expect("label state poisoned");
            state
                .assign(id, label.clone())
                .map_err(Status::invalid_argument)?;
        }
        let persisted = label.clone();
        self.persist(move |store| persist::save_torrent_label(store, id, persisted.as_deref()))
            .await?;
        self.apply_limits(id).await?;
        Ok(Response::new(SetTorrentLabelResponse {
            label: label.unwrap_or_default(),
        }))
    }

    async fn set_torrent_limits(
        &self,
        request: Request<SetTorrentLimitsRequest>,
    ) -> Result<Response<SetTorrentLimitsResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let limits = (!request.clear).then_some(TorrentLimits {
            download_rate_limit: request.download_rate_limit,
            upload_rate_limit: request.upload_rate_limit,
            connections_limit: request.connections_limit,
            upload_slots_limit: request.upload_slots_limit,
        });
        {
            let mut state = self.state.labels.lock().expect("label state poisoned");
            state.set_override(request.id, limits);
        }
        self.apply_limits(request.id).await?;
        Ok(Response::new(SetTorrentLimitsResponse {}))
    }

    async fn get_auto_add_status(
        &self,
        request: Request<GetAutoAddStatusRequest>,
    ) -> Result<Response<GetAutoAddStatusResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let config = self
            .state
            .config
            .resolved()
            .map_err(|error| Status::internal(error.to_string()))?;
        let runtime = self.state.autoadd_status();
        let dirs = config
            .autoadd
            .iter()
            .map(|watch| {
                let status = runtime
                    .iter()
                    .find(|(path, _)| *path == watch.path)
                    .map(|(_, status)| status);
                AutoAddDirStatus {
                    path: watch.path.display().to_string(),
                    enabled: watch.enabled,
                    label: watch.label.clone().unwrap_or_default(),
                    move_to: watch
                        .move_to
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    pending: status.map_or(0, |status| status.pending),
                    added: status.map_or(0, |status| status.added),
                    duplicates: status.map_or(0, |status| status.duplicates),
                    invalid: status.map_or(0, |status| status.invalid),
                    deferred: status.map_or(0, |status| status.deferred),
                    last_error: status
                        .and_then(|status| status.last_error.clone())
                        .unwrap_or_default(),
                    last_scan_unix: status
                        .and_then(|status| status.last_scan)
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |elapsed| {
                            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
                        }),
                }
            })
            .collect();
        Ok(Response::new(GetAutoAddStatusResponse { dirs }))
    }

    async fn scan_auto_add(
        &self,
        request: Request<ScanAutoAddRequest>,
    ) -> Result<Response<ScanAutoAddResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let path = request.into_inner().path;
        let totals = if path.is_empty() {
            self.state.scan_now().await
        } else {
            let path = PathBuf::from(&path);
            let labels = self
                .state
                .config
                .resolved()
                .map_err(|error| Status::internal(error.to_string()))?
                .autoadd;
            if !labels.iter().any(|watch| watch.path == path) {
                return Err(Status::not_found(format!(
                    "watch directory {path:?} is not configured"
                )));
            }
            self.state
                .scan_watch_now(&path)
                .await
                .ok_or_else(|| Status::not_found("watch directory vanished"))?
        };
        Ok(Response::new(ScanAutoAddResponse {
            added: totals.added,
            duplicates: totals.duplicates,
            failed: totals.failed,
            invalid: totals.invalid,
            deferred: totals.deferred,
        }))
    }

    async fn get_hook_status(
        &self,
        request: Request<GetHookStatusRequest>,
    ) -> Result<Response<GetHookStatusResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let limit = match request.into_inner().limit {
            0 => 20,
            limit => usize::try_from(limit).unwrap_or(20).min(200),
        };
        let store = Arc::clone(&self.state.store);
        let (pending, runs) = tokio::task::spawn_blocking(move || {
            let store = store.lock().expect("state store mutex poisoned");
            let pending = crate::events::pending_len(&**store).unwrap_or(0);
            let runs = crate::events::recent_runs(&**store, limit).unwrap_or_default();
            (pending, runs)
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        let runs = runs
            .into_iter()
            .map(|run| {
                let succeeded = run.succeeded();
                ProtoHookRun {
                    event: run.event.as_str().to_owned(),
                    torrent_id: run.torrent_id,
                    torrent_name: run.torrent_name,
                    command: run.command,
                    succeeded,
                    exit_code: run.exit_code.unwrap_or(0),
                    timed_out: run.timed_out,
                    duration_ms: run.duration_ms,
                    stdout: run.stdout,
                    stderr: run.stderr,
                    at_unix: run.at_unix,
                }
            })
            .collect();
        Ok(Response::new(GetHookStatusResponse {
            pending: u32::try_from(pending).unwrap_or(u32::MAX),
            runs,
        }))
    }

    async fn get_torrent(
        &self,
        request: Request<GetTorrentRequest>,
    ) -> Result<Response<GetTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let status = self
            .state
            .mirror
            .get(id)
            .ok_or_else(|| Status::not_found(format!("torrent {id} not found")))?;
        let mut torrent = to_proto_torrent(status);
        torrent.pause_reasons = self
            .state
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .reasons(id)
            .bits();
        torrent.label = self
            .state
            .labels
            .lock()
            .expect("label state poisoned")
            .label(id)
            .unwrap_or_default()
            .to_owned();
        Ok(Response::new(GetTorrentResponse {
            torrent: Some(torrent),
        }))
    }

    async fn list_torrents(
        &self,
        request: Request<ListTorrentsRequest>,
    ) -> Result<Response<ListTorrentsResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let query = TorrentQuery {
            filter: (!request.filter.is_empty()).then_some(request.filter),
            state: (request.state_filter != 0).then_some(request.state_filter),
            label: (!request.label_filter.is_empty()).then_some(request.label_filter),
            sort: sort_from_proto(request.sort),
            descending: request.descending,
            offset: request.offset as usize,
            limit: request.limit as usize,
        };
        let labels = self
            .state
            .labels
            .lock()
            .expect("label state poisoned")
            .assignments();
        let (torrents, total) = self.state.mirror.query_page_with_labels(&query, &labels);
        let reasons = self
            .state
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .snapshot();
        Ok(Response::new(ListTorrentsResponse {
            torrents: torrents
                .into_iter()
                .map(|status| {
                    let id = status.id;
                    let mut torrent = to_proto_torrent(status);
                    torrent.pause_reasons = reasons.get(&id).copied().unwrap_or_default().bits();
                    torrent.label = labels.get(&id).cloned().unwrap_or_default();
                    torrent
                })
                .collect(),
            total: total as u32,
        }))
    }

    async fn get_torrent_files(
        &self,
        request: Request<GetTorrentFilesRequest>,
    ) -> Result<Response<GetTorrentFilesResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let files = self.with_engine(move |engine| engine.files(id)).await?;
        Ok(Response::new(GetTorrentFilesResponse {
            files: files
                .into_iter()
                .map(|file| TorrentFile {
                    path: file.path,
                    size: file.size,
                    progress: file.progress,
                    priority: file.priority,
                })
                .collect(),
        }))
    }

    async fn get_torrent_peers(
        &self,
        request: Request<GetTorrentPeersRequest>,
    ) -> Result<Response<GetTorrentPeersResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let peers = self.with_engine(move |engine| engine.peers(id)).await?;
        Ok(Response::new(GetTorrentPeersResponse {
            peers: peers
                .into_iter()
                .map(|peer| TorrentPeer {
                    endpoint: peer.endpoint,
                    client: peer.client,
                    progress: peer.progress,
                    download_rate: peer.download_rate,
                    upload_rate: peer.upload_rate,
                })
                .collect(),
        }))
    }

    async fn get_torrent_trackers(
        &self,
        request: Request<GetTorrentTrackersRequest>,
    ) -> Result<Response<GetTorrentTrackersResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let trackers = self.with_engine(move |engine| engine.trackers(id)).await?;
        Ok(Response::new(GetTorrentTrackersResponse {
            trackers: trackers
                .into_iter()
                .map(|tracker| TorrentTracker {
                    url: tracker.url,
                    status: tracker.status,
                    seeds: tracker.seeds,
                    leechers: tracker.leechers,
                })
                .collect(),
        }))
    }

    async fn get_status(
        &self,
        request: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let status = self.state.aggregate_status();
        let scheduler = self.state.scheduler_status();
        Ok(Response::new(GetStatusResponse {
            torrent_count: status.torrent_count,
            download_rate: status.download_rate,
            upload_rate: status.upload_rate,
            free_space_bytes: status.free_space,
            downloading_count: status.downloading,
            seeding_count: status.seeding,
            queued_count: status.queued,
            paused_count: status.paused,
            error_count: status.error,
            checking_count: status.checking,
            active_count: status.active,
            moving_count: status.moving,
            scheduler_enabled: self
                .state
                .config
                .resolved()
                .is_ok_and(|config| config.scheduler.enabled),
            scheduler_level: scheduler
                .level
                .map_or_else(String::new, |level| level.label().to_owned()),
            scheduler_next_unix: scheduler.next_unix,
        }))
    }

    async fn get_notification_status(
        &self,
        request: Request<GetNotificationStatusRequest>,
    ) -> Result<Response<GetNotificationStatusResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let status = self
            .state
            .notification_status()
            .await
            .map_err(Status::internal)?;
        let channels = self.state.config.resolved().map_or(0, |config| {
            config
                .notify
                .webhook
                .iter()
                .filter(|rule| rule.enabled)
                .count()
        });
        Ok(Response::new(GetNotificationStatusResponse {
            pending: status.pending,
            dead: status.dead,
            channels: u32::try_from(channels).unwrap_or(u32::MAX),
        }))
    }

    async fn get_disks(
        &self,
        request: Request<GetDisksRequest>,
    ) -> Result<Response<GetDisksResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let config = self
            .state
            .config
            .resolved()
            .map_err(|error| Status::internal(error.to_string()))?;
        let report = match self.state.scan_storage().await {
            Some(report) => report,
            None => self
                .state
                .storage_payload()
                .ok_or_else(|| Status::unavailable("the storage scan did not finish"))?,
        };
        // The panel offers every filesystem; the guard only consults zones.
        let min_free = config.diskguard.min_free_bytes;
        let report = tokio::task::spawn_blocking(move || {
            let mut report = report;
            let mounts = crate::diskguard::mount_table();
            crate::diskguard::include_unzoned(
                &mut report,
                &mounts,
                min_free,
                crate::diskguard::disk_stat,
            );
            report
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        Ok(Response::new(GetDisksResponse {
            disks: report
                .disks
                .iter()
                .map(|disk| ProtoDisk {
                    mount: disk.mount.display().to_string(),
                    device: disk.device.clone(),
                    fs: disk.fstype.clone(),
                    total_bytes: disk.total,
                    free_bytes: disk.free,
                    used_bytes: disk.used,
                    low: disk.low,
                    torrents: u32::try_from(disk.torrents).unwrap_or(u32::MAX),
                    labels: disk.labels.clone(),
                    zones: disk
                        .zones
                        .iter()
                        .map(|zone| ProtoDiskZone {
                            path: zone.path.display().to_string(),
                            label: zone.label.clone().unwrap_or_default(),
                            staging: zone.staging,
                            enabled: zone.enabled,
                        })
                        .collect(),
                })
                .collect(),
            unknown: report
                .unknown
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            total_free_bytes: report.total_free(),
            min_free_bytes: config.diskguard.min_free_bytes,
            margin_bytes: config.diskguard.margin_bytes,
            enabled: config.diskguard.enabled,
        }))
    }

    async fn send_test_notification(
        &self,
        request: Request<SendTestNotificationRequest>,
    ) -> Result<Response<SendTestNotificationResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let queued = self
            .state
            .send_test_notification()
            .await
            .map_err(Status::internal)?;
        Ok(Response::new(SendTestNotificationResponse { queued }))
    }

    async fn get_config(
        &self,
        request: Request<GetConfigRequest>,
    ) -> Result<Response<GetConfigResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let resolved = self.state.config.resolved().map_err(config_status)?;
        Ok(Response::new(GetConfigResponse {
            toml: self.state.config.toml(),
            revision: self.state.config.revision(),
            env_locked: resolved.env_locked.into_iter().collect(),
        }))
    }

    async fn set_config(
        &self,
        request: Request<SetConfigRequest>,
    ) -> Result<Response<SetConfigResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let revision = self
            .state
            .config
            .apply_toml(&request.toml, request.revision)
            .map_err(config_status)?;
        let resolved = self.state.config.resolved().map_err(config_status)?;
        let settings = EngineSettings {
            download_rate_limit: resolved.session.download_rate_limit,
            upload_rate_limit: resolved.session.upload_rate_limit,
            seed_mode: resolved.session.seed_mode,
            active_downloads: resolved.session.active_downloads,
            active_seeds: resolved.session.active_seeds,
            active_limit: resolved.session.active_limit,
        };
        self.with_engine(move |engine| {
            engine.set_settings(settings);
            Ok(())
        })
        .await?;
        let _ = self.state.config_watch.send(revision);
        Ok(Response::new(SetConfigResponse { revision }))
    }

    type WatchStream = Pin<Box<dyn Stream<Item = Result<WatchResponse, Status>> + Send + 'static>>;

    async fn watch(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        self.state.auth.check(request.metadata())?;
        let torrents =
            BroadcastStream::new(self.state.watch.subscribe()).filter_map(|item| match item {
                Ok(event) => Some(Ok(to_proto_watch(event))),
                Err(error) => Some(Err(Status::resource_exhausted(error.to_string()))),
            });
        let configs = BroadcastStream::new(self.state.config_watch.subscribe()).filter_map(
            |item| match item {
                Ok(revision) => Some(Ok(WatchResponse {
                    event: Some(WatchEvent::ConfigChanged(ConfigChanged { revision })),
                })),
                Err(error) => Some(Err(Status::resource_exhausted(error.to_string()))),
            },
        );
        Ok(Response::new(Box::pin(torrents.merge(configs))))
    }
}

/// Starts the TLS gRPC server on an already-bound listener.
pub async fn serve(
    listener: TcpListener,
    state: Arc<AppState>,
    tls: &TlsMaterial,
) -> Result<(), tonic::transport::Error> {
    let _engine_watcher = spawn_engine_watcher(Arc::clone(&state));
    crate::autoadd::spawn_autoadd(Arc::clone(&state));
    crate::hooks::spawn_hook_runner(Arc::clone(&state));
    crate::scheduler::spawn_scheduler(Arc::clone(&state));
    crate::notify::spawn_notifier(Arc::clone(&state));
    crate::diskguard::moves::spawn_move_retry(Arc::clone(&state));
    crate::diskguard::guard::spawn_disk_guard(Arc::clone(&state));
    let tls_config = ServerTlsConfig::new().identity(Identity::from_pem(
        tls.cert_pem.as_bytes(),
        tls.key_pem.as_bytes(),
    ));
    Server::builder()
        .tls_config(tls_config)?
        .add_service(FerrentServer::new(FerrentService::new(state)))
        .serve_with_incoming(TcpListenerStream::new(listener))
        .await
}

/// Journals `Finished`/`Error` events and moves finished torrents to their
/// label's `move_completed_path`.
pub fn spawn_engine_watcher(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = state.watch.subscribe();
        loop {
            match events.recv().await {
                Ok(EngineEvent::Finished { id }) => {
                    if let Some(status) = state.mirror.get(id) {
                        let event = crate::events::NewEvent::new(
                            crate::events::EventKind::Finished,
                            id,
                            status.name.clone(),
                        )
                        .with_path(status.save_path.display().to_string())
                        .with_infohash(status.infohash.clone());
                        if let Err(error) = state.publish_event(event).await {
                            eprintln!("hook journal: {error}");
                        }
                    }
                    state.handle_finished_move(id).await;
                }
                Ok(EngineEvent::StateChanged {
                    id,
                    state: EngineState::Error(message),
                }) => {
                    let name = state
                        .mirror
                        .get(id)
                        .map(|status| status.name.clone())
                        .unwrap_or_default();
                    let event =
                        crate::events::NewEvent::new(crate::events::EventKind::Error, id, name)
                            .with_error(Some(message));
                    if let Err(error) = state.publish_event(event).await {
                        eprintln!("hook journal: {error}");
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    eprintln!("engine watcher lagged by {skipped} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn engine_status(error: EngineError) -> Status {
    match &error {
        EngineError::AlreadyExists { .. } => Status::already_exists(error.to_string()),
        EngineError::NotFound { .. } => Status::not_found(error.to_string()),
        EngineError::InvalidTransition { .. } => Status::failed_precondition(error.to_string()),
        EngineError::Rejected { .. } => Status::invalid_argument(error.to_string()),
    }
}

fn config_status(error: ConfigError) -> Status {
    match &error {
        ConfigError::RevisionConflict { .. } => Status::aborted(error.to_string()),
        ConfigError::Parse(_) | ConfigError::Invalid(_) => {
            Status::invalid_argument(error.to_string())
        }
        ConfigError::Io { .. } => Status::internal(error.to_string()),
    }
}

/// Resolves a client-supplied save path against the daemon download directory.
///
/// An empty path means "the daemon default". Relative paths are anchored to
/// that directory so the result never depends on the daemon's working
/// directory.
pub(crate) fn resolve_save_path(download_dir: &Path, requested: &str) -> PathBuf {
    let requested = requested.trim();
    if requested.is_empty() {
        return download_dir.to_path_buf();
    }
    let path = Path::new(requested);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        download_dir.join(path)
    }
}

fn to_proto_state(state: &EngineState) -> TorrentState {
    match state {
        EngineState::Queued => TorrentState::Queued,
        EngineState::Checking => TorrentState::Checking,
        EngineState::Downloading => TorrentState::Downloading,
        EngineState::Seeding => TorrentState::Seeding,
        EngineState::Paused => TorrentState::Paused,
        EngineState::Moving => TorrentState::Moving,
        EngineState::Error(_) => TorrentState::Error,
    }
}

fn to_proto_torrent(status: TorrentStatus) -> Torrent {
    let error = if let EngineState::Error(message) = &status.state {
        message.clone()
    } else {
        String::new()
    };
    Torrent {
        id: status.id,
        infohash: status.infohash,
        name: status.name,
        save_path: status.save_path.to_string_lossy().into_owned(),
        total_bytes: status.total_bytes,
        state: to_proto_state(&status.state) as i32,
        progress: status.progress,
        download_rate: status.download_rate,
        upload_rate: status.upload_rate,
        seeds: status.seeds,
        peers: status.peers,
        eta_seconds: status.eta_seconds,
        error,
        queue_position: status.queue_position,
        pause_reasons: 0,
        label: String::new(),
    }
}

fn to_proto_watch(event: EngineEvent) -> WatchResponse {
    let event = match event {
        EngineEvent::Added(status) => WatchEvent::Added(to_proto_torrent(status)),
        EngineEvent::StateChanged { id, state } => WatchEvent::StateChanged(StateChanged {
            id,
            state: to_proto_state(&state) as i32,
        }),
        EngineEvent::Progress {
            id,
            progress,
            download_rate,
            upload_rate,
            queue_position,
        } => WatchEvent::Progress(Progress {
            id,
            progress,
            download_rate,
            upload_rate,
            queue_position,
        }),
        EngineEvent::Removed { id } => WatchEvent::Removed(Removed { id }),
        EngineEvent::Finished { id } => WatchEvent::Finished(TorrentFinished { id }),
    };
    WatchResponse { event: Some(event) }
}

fn label_to_proto(label: &DomainLabel) -> ProtoLabel {
    ProtoLabel {
        name: label.name.clone(),
        color: label.color.clone(),
        move_completed_path: label
            .move_completed_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        download_rate_limit: label.download_rate_limit,
        upload_rate_limit: label.upload_rate_limit,
        connections_limit: label.connections_limit,
        upload_slots_limit: label.upload_slots_limit,
        match_trackers: label.match_trackers.clone(),
        match_name: label.match_name.clone().unwrap_or_default(),
    }
}

fn label_from_proto(label: ProtoLabel) -> Result<DomainLabel, Status> {
    let move_completed_path = label.move_completed_path.trim();
    let match_name = label.match_name.trim();
    let label = DomainLabel {
        name: DomainLabel::normalize_name(&label.name),
        color: label.color.trim().to_owned(),
        move_completed_path: (!move_completed_path.is_empty())
            .then(|| PathBuf::from(move_completed_path)),
        download_rate_limit: label.download_rate_limit,
        upload_rate_limit: label.upload_rate_limit,
        connections_limit: label.connections_limit,
        upload_slots_limit: label.upload_slots_limit,
        match_trackers: label.match_trackers,
        match_name: (!match_name.is_empty()).then(|| match_name.to_owned()),
    };
    label.validate().map_err(Status::invalid_argument)?;
    Ok(label)
}

fn sort_from_proto(value: i32) -> TorrentSort {
    match SortField::try_from(value).unwrap_or(SortField::Unspecified) {
        SortField::Name => TorrentSort::Name,
        SortField::Progress => TorrentSort::Progress,
        SortField::DownloadRate => TorrentSort::DownloadRate,
        SortField::UploadRate => TorrentSort::UploadRate,
        SortField::Queue => TorrentSort::Queue,
        SortField::Label => TorrentSort::Label,
        SortField::Id | SortField::Unspecified => TorrentSort::Id,
    }
}

fn direction_from_proto(value: i32) -> ferrent_engine::QueueDirection {
    match ProtoQueueDirection::try_from(value).unwrap_or(ProtoQueueDirection::Unspecified) {
        ProtoQueueDirection::Down => ferrent_engine::QueueDirection::Down,
        ProtoQueueDirection::Up | ProtoQueueDirection::Unspecified => {
            ferrent_engine::QueueDirection::Up
        }
    }
}

/// Spawns the mirror task for a freshly constructed engine.
pub fn spawn_mirror(
    mirror: Arc<StateMirror>,
    events: broadcast::Receiver<EngineEvent>,
    watch: broadcast::Sender<EngineEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_mirror(mirror, events, watch))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use ferrent_engine::{AddTorrent, MockEngine, TorrentEngine, TorrentSource};
    use tokio::sync::broadcast;

    use super::*;
    use crate::config::ConfigStore;
    use crate::control::{PauseReason, TorrentControl};
    use crate::labels::{Label, LabelState};
    use crate::state::StateMirror;
    use crate::store::MemoryStore;

    #[test]
    fn empty_save_path_uses_the_daemon_default() {
        let dir = std::path::Path::new("/srv/downloads");
        assert_eq!(resolve_save_path(dir, ""), dir.to_path_buf());
        assert_eq!(resolve_save_path(dir, "   "), dir.to_path_buf());
    }

    #[test]
    fn relative_save_path_is_anchored_to_the_download_dir() {
        let dir = std::path::Path::new("/srv/downloads");
        assert_eq!(
            resolve_save_path(dir, "movies"),
            std::path::PathBuf::from("/srv/downloads/movies")
        );
    }

    #[test]
    fn absolute_save_path_is_kept() {
        let dir = std::path::Path::new("/srv/downloads");
        assert_eq!(
            resolve_save_path(dir, "/mnt/other"),
            std::path::PathBuf::from("/mnt/other")
        );
    }

    fn authorized<T>(message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert("authorization", "Bearer token".parse().expect("metadata"));
        request
    }

    #[tokio::test]
    async fn auto_add_status_and_scan_over_rpc() {
        let base = std::env::temp_dir().join(format!("ferrent-rpc-autoadd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let watch = base.join("watch");
        std::fs::create_dir_all(&watch).expect("watch dir");
        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!(
                    "[[autoadd]]\npath = {:?}\nlabel = \"movies\"\n",
                    watch.display().to_string()
                ),
                0,
            )
            .expect("config");

        let mut labels = LabelState::default();
        labels
            .upsert(Label {
                name: "movies".to_owned(),
                ..Label::default()
            })
            .expect("label");
        let (send, _) = broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            send,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            labels,
            "token",
            base.clone(),
        ));
        let service = FerrentService::new(Arc::clone(&state));

        let status = service
            .get_auto_add_status(authorized(GetAutoAddStatusRequest {}))
            .await
            .expect("status")
            .into_inner();
        assert_eq!(status.dirs.len(), 1);
        assert_eq!(status.dirs[0].path, watch.display().to_string());
        assert_eq!(status.dirs[0].label, "movies");
        assert_eq!(status.dirs[0].added, 0);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"d4:infod6:lengthi7e4:name4:showee");
        std::fs::write(watch.join("a.torrent"), bytes).expect("write");
        let totals = service
            .scan_auto_add(authorized(ScanAutoAddRequest {
                path: String::new(),
            }))
            .await
            .expect("scan")
            .into_inner();
        assert_eq!(totals.added, 1, "{totals:?}");

        let status = service
            .get_auto_add_status(authorized(GetAutoAddStatusRequest {}))
            .await
            .expect("status")
            .into_inner();
        assert_eq!(status.dirs[0].added, 1);
        assert!(status.dirs[0].last_scan_unix > 0);
        assert!(status.dirs[0].last_error.is_empty());

        let missing = service
            .scan_auto_add(authorized(ScanAutoAddRequest {
                path: "/nope".to_owned(),
            }))
            .await;
        assert_eq!(
            missing.expect_err("not found").code(),
            tonic::Code::NotFound
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn label_mover_moves_a_finished_torrent() {
        let mut engine = MockEngine::new();
        let id = engine
            .add_torrent(AddTorrent {
                name: "movie".to_owned(),
                total_bytes: 1,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:movie".to_owned()),
                save_path: PathBuf::from("/tmp/source"),
                resume_data: None,
            })
            .expect("add");
        let mut labels = LabelState::default();
        let target = PathBuf::from("/tmp/target");
        labels
            .upsert(Label {
                name: "movies".to_owned(),
                move_completed_path: Some(target.clone()),
                ..Label::default()
            })
            .expect("label");
        labels
            .assign(id, Some("movies".to_owned()))
            .expect("assign");
        let (watch, _) = broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(engine),
            Arc::new(StateMirror::new()),
            watch,
            Box::new(MemoryStore::new()),
            ConfigStore::default(),
            TorrentControl::default(),
            labels,
            "token",
            PathBuf::from("."),
        ));
        let _watcher = spawn_engine_watcher(Arc::clone(&state));
        tokio::time::sleep(Duration::from_millis(50)).await;
        state
            .watch
            .send(EngineEvent::Finished { id })
            .expect("finished event");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let status = state
                .engine
                .lock()
                .expect("engine")
                .status(id)
                .expect("status");
            if status.save_path == target {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "label mover did not move the payload: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn bulk_reasons_pause_and_resume_through_the_engine() {
        let mut engine = MockEngine::new();
        let id = engine
            .add_torrent(AddTorrent {
                name: "bulk".to_owned(),
                total_bytes: 1,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:bulk".to_owned()),
                save_path: PathBuf::from("/tmp"),
                resume_data: None,
            })
            .expect("add");
        let (watch, _) = broadcast::channel(8);
        let state = AppState::new(
            Box::new(engine),
            Arc::new(StateMirror::new()),
            watch,
            Box::new(MemoryStore::new()),
            ConfigStore::default(),
            TorrentControl::default(),
            LabelState::default(),
            "token",
            PathBuf::from("."),
        );

        assert_eq!(
            state
                .set_pause_reason(&[id], PauseReason::Scheduler, true)
                .await
                .expect("pause"),
            1
        );
        assert_eq!(
            state
                .engine
                .lock()
                .expect("engine")
                .status(id)
                .expect("status")
                .state,
            EngineState::Paused
        );

        assert_eq!(
            state
                .set_pause_reason(&[id], PauseReason::Scheduler, false)
                .await
                .expect("resume"),
            1
        );
        assert_ne!(
            state
                .engine
                .lock()
                .expect("engine")
                .status(id)
                .expect("status")
                .state,
            EngineState::Paused
        );
    }

    #[tokio::test]
    async fn adding_with_a_label_writes_into_its_staging_folder() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("ferrent-add-zone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let staging = base.join("_progress");
        let library = base.join("Filmek");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::create_dir_all(&library).expect("library");
        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!(
                    r#"
download_dir = {downloads:?}

[[diskguard.zone]]
path = {library:?}
label = "filmek"

[[diskguard.zone]]
path = {staging:?}
staging = true
"#,
                    downloads = base.join("downloads").display().to_string(),
                    library = library.display().to_string(),
                    staging = staging.display().to_string(),
                ),
                0,
            )
            .expect("config");
        let mut labels = LabelState::default();
        labels
            .upsert(Label {
                name: "filmek".to_owned(),
                ..Label::default()
            })
            .expect("label");
        let (watch, _) = broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            watch,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            labels,
            "token",
            base.clone(),
        ));
        let service = FerrentService::new(Arc::clone(&state));
        let added = service
            .add_torrent(authorized(AddTorrentRequest {
                name: "film.mkv".to_owned(),
                metainfo: Vec::new(),
                save_path: String::new(),
                total_bytes: 1,
                label: "Filmek".to_owned(),
                magnet: String::new(),
            }))
            .await
            .expect("add")
            .into_inner();
        assert_eq!(
            state
                .engine
                .lock()
                .expect("engine")
                .status(added.id)
                .expect("status")
                .save_path,
            staging
        );
        assert_eq!(
            state.labels.lock().expect("labels").label(added.id),
            Some("filmek")
        );

        // The same add is refused when the staging disk is below the floor.
        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!("[diskguard]\nmin_free_bytes = {}\n", u64::MAX / 2),
                0,
            )
            .expect("config");
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            broadcast::channel(8).0,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            LabelState::default(),
            "token",
            base.clone(),
        ));
        let service = FerrentService::new(state);
        let error = service
            .add_torrent(authorized(AddTorrentRequest {
                name: "low.mkv".to_owned(),
                metainfo: Vec::new(),
                save_path: String::new(),
                total_bytes: 1,
                label: String::new(),
                magnet: String::new(),
            }))
            .await
            .expect_err("low disk");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition, "{error}");
        assert!(error.message().contains("required"), "{error}");

        // The preallocate gate refuses a payload that cannot fit.
        let config = ConfigStore::default();
        config
            .apply_toml(
                "[diskguard]\nrequire_space_for_torrent = true\nmargin_bytes = 0\n",
                0,
            )
            .expect("config");
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            broadcast::channel(8).0,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            LabelState::default(),
            "token",
            base.clone(),
        ));
        let service = FerrentService::new(state);
        let error = service
            .add_torrent(authorized(AddTorrentRequest {
                name: "huge.mkv".to_owned(),
                metainfo: Vec::new(),
                save_path: String::new(),
                total_bytes: u64::MAX / 2,
                label: String::new(),
                magnet: String::new(),
            }))
            .await
            .expect_err("payload does not fit");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition, "{error}");
        assert!(error.message().contains("payload"), "{error}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn disks_report_the_configured_zones() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("ferrent-disks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let staging = base.join("_progress");
        let library = base.join("Filmek");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::create_dir_all(&library).expect("library");
        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!(
                    r#"
[diskguard]
enabled = true
min_free_bytes = 1048576
margin_bytes = 1048576

[[diskguard.zone]]
path = {library:?}
label = "filmek"

[[diskguard.zone]]
path = {staging:?}
staging = true
"#,
                    library = library.display().to_string(),
                    staging = staging.display().to_string(),
                ),
                0,
            )
            .expect("config");
        let (watch, _) = broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            watch,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            LabelState::default(),
            "token",
            base.clone(),
        ));
        let service = FerrentService::new(Arc::clone(&state));
        let disks = service
            .get_disks(authorized(GetDisksRequest {}))
            .await
            .expect("disks")
            .into_inner();

        assert!(disks.enabled);
        assert_eq!(disks.min_free_bytes, 1 << 20);
        assert_eq!(disks.margin_bytes, 1 << 20);
        // Every real filesystem is offered; the zoned one is ours.
        let disk = disks
            .disks
            .iter()
            .find(|disk| !disk.zones.is_empty())
            .expect("zoned disk");
        assert!(disk.mount.starts_with('/'), "{disk:?}");
        assert!(disk.free_bytes > 0, "{disk:?}");
        assert_eq!(disk.zones.len(), 2);
        assert_eq!(disk.labels, vec!["filmek".to_owned()]);
        assert_eq!(disk.zones[1].path, staging.display().to_string());
        assert!(disk.zones[1].staging);
        assert!(disks.total_free_bytes >= disk.free_bytes);
        assert!(!disk.low, "plenty of space");

        let _ = std::fs::remove_dir_all(&base);
    }
}
