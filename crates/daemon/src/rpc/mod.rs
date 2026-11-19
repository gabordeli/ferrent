//! gRPC server: TLS transport, bearer-token auth, and the `Ferrent` service.

pub mod tls;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ferrent_engine::{
    AddTorrent, EngineError, EngineEvent, TorrentEngine, TorrentSource,
    TorrentState as EngineState, TorrentStatus,
};
use ferrent_protocol::v1::ferrent_server::{Ferrent, FerrentServer};
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ferrent_protocol::v1::{
    AddTorrentRequest, AddTorrentResponse, GetStatusRequest, GetStatusResponse,
    GetTorrentFilesRequest, GetTorrentFilesResponse, GetTorrentPeersRequest,
    GetTorrentPeersResponse, GetTorrentRequest, GetTorrentResponse,
    GetTorrentTrackersRequest, GetTorrentTrackersResponse, ListTorrentsRequest,
    ListTorrentsResponse, MoveTorrentRequest, MoveTorrentResponse, PauseTorrentRequest,
    PauseTorrentResponse, Progress, QueueDirection as ProtoQueueDirection, RemoveTorrentRequest,
    RemoveTorrentResponse, Removed, ResumeTorrentRequest, ResumeTorrentResponse, SortField,
    StateChanged, Torrent, TorrentFile, TorrentPeer, TorrentState, TorrentTracker, WatchRequest,
    WatchResponse,
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
    pub active: u32,
}

/// Shared daemon state used by the RPC service.
pub struct AppState {
    engine: Arc<Mutex<Box<dyn TorrentEngine>>>,
    mirror: Arc<StateMirror>,
    watch: broadcast::Sender<EngineEvent>,
    store: Arc<Mutex<Box<dyn StateStore>>>,
    auth: Auth,
    data_dir: PathBuf,
    download_dir: PathBuf,
}

impl AppState {
    #[must_use]
    pub fn new(
        engine: Box<dyn TorrentEngine>,
        mirror: Arc<StateMirror>,
        watch: broadcast::Sender<EngineEvent>,
        store: Box<dyn StateStore>,
        token: impl Into<String>,
        data_dir: PathBuf,
        download_dir: PathBuf,
    ) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            mirror,
            watch,
            store: Arc::new(Mutex::new(store)),
            auth: Auth::new(token),
            data_dir,
            download_dir,
        }
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
        let save_path = spec.save_path.clone();
        let metainfo = match &spec.source {
            TorrentSource::Metainfo(bytes) => bytes.clone(),
            TorrentSource::Magnet(_) => Vec::new(),
        };
        let id = self
            .with_engine(move |engine| engine.add_torrent(spec))
            .await?;
        let persisted = PersistedTorrent {
            id,
            name,
            save_path,
            metainfo,
            resume_data: None,
        };
        self.persist(move |store| persist::save_torrent(store, &persisted))
            .await?;
        Ok(Response::new(AddTorrentResponse { id }))
    }

    async fn pause_torrent(
        &self,
        request: Request<PauseTorrentRequest>,
    ) -> Result<Response<PauseTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        self.with_engine(move |engine| engine.pause(id)).await?;
        Ok(Response::new(PauseTorrentResponse {}))
    }

    async fn resume_torrent(
        &self,
        request: Request<ResumeTorrentRequest>,
    ) -> Result<Response<ResumeTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        self.with_engine(move |engine| engine.resume(id)).await?;
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
        Ok(Response::new(GetTorrentResponse {
            torrent: Some(to_proto_torrent(status)),
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
            sort: sort_from_proto(request.sort),
            descending: request.descending,
            offset: request.offset as usize,
            limit: request.limit as usize,
        };
        let (torrents, total) = self.state.mirror.query_page(&query);
        Ok(Response::new(ListTorrentsResponse {
            torrents: torrents.into_iter().map(to_proto_torrent).collect(),
            total: total as u32,
        }))
    }

    async fn get_torrent_files(
        &self,
        request: Request<GetTorrentFilesRequest>,
    ) -> Result<Response<GetTorrentFilesResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let id = request.into_inner().id;
        let files = self
            .with_engine(move |engine| engine.files(id))
            .await?;
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
        let peers = self
            .with_engine(move |engine| engine.peers(id))
            .await?;
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
        let trackers = self
            .with_engine(move |engine| engine.trackers(id))
            .await?;
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
        }))
    }

    type WatchStream = Pin<Box<dyn Stream<Item = Result<WatchResponse, Status>> + Send + 'static>>;

    async fn watch(
        &self,
        request: Request<WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        self.state.auth.check(request.metadata())?;
        let receiver = self.state.watch.subscribe();
        let stream = BroadcastStream::new(receiver).filter_map(|item| match item {
            Ok(event) => Some(Ok(to_proto_watch(event))),
            Err(error) => Some(Err(Status::resource_exhausted(error.to_string()))),
        });
        Ok(Response::new(Box::pin(stream)))
    }
}

/// Starts the TLS gRPC server on an already-bound listener.
pub async fn serve(
    listener: TcpListener,
    state: Arc<AppState>,
    tls: &TlsMaterial,
) -> Result<(), tonic::transport::Error> {
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

fn engine_status(error: EngineError) -> Status {
    match &error {
        EngineError::NotFound { .. } => Status::not_found(error.to_string()),
        EngineError::InvalidTransition { .. } => Status::failed_precondition(error.to_string()),
        EngineError::Rejected { .. } => Status::invalid_argument(error.to_string()),
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
    };
    WatchResponse { event: Some(event) }
}

fn sort_from_proto(value: i32) -> TorrentSort {
    match SortField::try_from(value).unwrap_or(SortField::Unspecified) {
        SortField::Name => TorrentSort::Name,
        SortField::Progress => TorrentSort::Progress,
        SortField::DownloadRate => TorrentSort::DownloadRate,
        SortField::UploadRate => TorrentSort::UploadRate,
        SortField::Queue => TorrentSort::Queue,
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
    use super::resolve_save_path;

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
}
