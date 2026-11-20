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
    AddTorrentRequest, AddTorrentResponse, ConfigChanged, CreateLabelRequest, DeleteLabelRequest,
    DeleteLabelResponse, GetConfigRequest, GetConfigResponse, GetStatusRequest, GetStatusResponse,
    GetTorrentFilesRequest, GetTorrentFilesResponse, GetTorrentPeersRequest,
    GetTorrentPeersResponse, GetTorrentRequest, GetTorrentResponse, GetTorrentTrackersRequest,
    GetTorrentTrackersResponse, Label as ProtoLabel, LabelEntry, LabelResponse, ListLabelsRequest,
    ListLabelsResponse, ListTorrentsRequest, ListTorrentsResponse, MoveTorrentRequest,
    MoveTorrentResponse, PauseTorrentRequest, PauseTorrentResponse, Progress,
    QueueDirection as ProtoQueueDirection, RemoveTorrentRequest, RemoveTorrentResponse, Removed,
    ResumeTorrentRequest, ResumeTorrentResponse, SetConfigRequest, SetConfigResponse,
    SetTorrentLabelRequest, SetTorrentLabelResponse, SetTorrentLimitsRequest,
    SetTorrentLimitsResponse, SortField, StateChanged, Torrent, TorrentFile, TorrentFinished,
    TorrentPeer, TorrentState, TorrentTracker, UpdateLabelRequest, WatchRequest, WatchResponse,
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
    engine: Arc<Mutex<Box<dyn TorrentEngine>>>,
    mirror: Arc<StateMirror>,
    watch: broadcast::Sender<EngineEvent>,
    config: ConfigStore,
    config_watch: broadcast::Sender<u64>,
    control: Arc<Mutex<TorrentControl>>,
    labels: Arc<Mutex<LabelState>>,
    store: Arc<Mutex<Box<dyn StateStore>>>,
    auth: Auth,
    data_dir: PathBuf,
    download_dir: PathBuf,
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
        download_dir: PathBuf,
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
            download_dir,
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
        let save_path = resolve_save_path(&self.state.download_dir, &request.save_path);
        let spec = AddTorrent {
            name: request.name,
            total_bytes: request.total_bytes,
            source: TorrentSource::Metainfo(request.metainfo),
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
                    Ok((id, trackers))
                }
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        let (id, trackers, duplicate) = match result {
            Ok((id, trackers)) => (id, trackers, false),
            Err(EngineError::AlreadyExists { id }) => (id, Vec::new(), true),
            Err(error) => return Err(engine_status(error)),
        };
        if !duplicate {
            let persisted = PersistedTorrent {
                id,
                name,
                save_path,
                metainfo,
                resume_data: None,
                paused_by_user: false,
                label: None,
            };
            self.persist(move |store| persist::save_torrent(store, &persisted))
                .await?;
            self.match_and_assign(id, &match_name, &trackers).await?;
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
        }))
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
    spawn_label_mover(Arc::clone(&state));
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

/// Moves finished torrents to their label's `move_completed_path`.
fn spawn_label_mover(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut events = state.watch.subscribe();
        loop {
            match events.recv().await {
                Ok(EngineEvent::Finished { id }) => {
                    let target = {
                        let labels = state.labels.lock().expect("label state poisoned");
                        labels.move_target(id)
                    };
                    let Some(target) = target else {
                        continue;
                    };
                    if state
                        .mirror
                        .get(id)
                        .is_some_and(|status| status.save_path == target)
                    {
                        continue;
                    }
                    let engine = Arc::clone(&state.engine);
                    let result = tokio::task::spawn_blocking(move || {
                        let mut engine = engine.lock().expect("engine mutex poisoned");
                        engine.move_storage(id, target)
                    })
                    .await;
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => eprintln!("label move failed for torrent {id}: {error}"),
                        Err(error) => eprintln!("label move task failed for torrent {id}: {error}"),
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    eprintln!("label mover lagged by {skipped} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
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
fn resolve_save_path(download_dir: &Path, requested: &str) -> PathBuf {
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
            PathBuf::from("/tmp"),
        ));
        spawn_label_mover(Arc::clone(&state));
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
            PathBuf::from("/tmp"),
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
}
