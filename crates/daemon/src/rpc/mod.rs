//! gRPC server: TLS transport, bearer-token auth, and the `Ferrent` service.

pub mod tls;

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ferrent_engine::{
    AddTorrent, EngineError, EngineEvent, TorrentEngine, TorrentSource,
    TorrentState as EngineState, TorrentStatus,
};
use ferrent_protocol::v1::ferrent_server::{Ferrent, FerrentServer};
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ferrent_protocol::v1::{
    AddTorrentRequest, AddTorrentResponse, GetTorrentRequest, GetTorrentResponse,
    ListTorrentsRequest, ListTorrentsResponse, PauseTorrentRequest, PauseTorrentResponse, Progress,
    RemoveTorrentRequest, RemoveTorrentResponse, Removed, ResumeTorrentRequest,
    ResumeTorrentResponse, SortField, StateChanged, Torrent, TorrentState, WatchRequest,
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

use crate::state::{StateMirror, TorrentQuery, TorrentSort, run_mirror};

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

/// Shared daemon state used by the RPC service.
pub struct AppState {
    engine: Arc<Mutex<Box<dyn TorrentEngine>>>,
    mirror: Arc<StateMirror>,
    watch: broadcast::Sender<EngineEvent>,
    auth: Auth,
}

impl AppState {
    #[must_use]
    pub fn new(
        engine: Box<dyn TorrentEngine>,
        mirror: Arc<StateMirror>,
        watch: broadcast::Sender<EngineEvent>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            mirror,
            watch,
            auth: Auth::new(token),
        }
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
}

#[tonic::async_trait]
impl Ferrent for FerrentService {
    async fn add_torrent(
        &self,
        request: Request<AddTorrentRequest>,
    ) -> Result<Response<AddTorrentResponse>, Status> {
        self.state.auth.check(request.metadata())?;
        let request = request.into_inner();
        let spec = AddTorrent {
            name: request.name,
            total_bytes: request.total_bytes,
            source: TorrentSource::Metainfo(request.metainfo),
            save_path: PathBuf::from(request.save_path),
        };
        let id = self
            .with_engine(move |engine| engine.add_torrent(spec))
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
        let id = request.into_inner().id;
        self.with_engine(move |engine| engine.remove(id)).await?;
        Ok(Response::new(RemoveTorrentResponse {}))
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
        total_bytes: status.total_bytes,
        state: to_proto_state(&status.state) as i32,
        progress: status.progress,
        download_rate: status.download_rate,
        upload_rate: status.upload_rate,
        error,
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
        } => WatchEvent::Progress(Progress {
            id,
            progress,
            download_rate,
            upload_rate,
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
        SortField::Id | SortField::Unspecified => TorrentSort::Id,
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
