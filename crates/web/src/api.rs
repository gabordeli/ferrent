//! REST facade over the daemon RPCs.
//!
//! The endpoints mirror the gRPC surface one-to-one; the only extra logic is
//! JSON shaping, query parsing and the HTTP status mapping. Every handler
//! requires a session, and mutating handlers respect `read_only`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, CreateLabelRequest, DeleteLabelRequest, GetStatusRequest,
    GetTorrentFilesRequest, GetTorrentRequest, Label, ListLabelsRequest, ListTorrentsRequest,
    PauseTorrentRequest, RemoveTorrentRequest, ResumeTorrentRequest, SetTorrentLabelRequest,
    SortField, Torrent, TorrentState,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, Session, sse};

/// Largest page the gateway serves.
pub const MAX_LIMIT: u32 = 1_000;
/// Largest bulk request.
pub const MAX_BULK: usize = 1_000;

/// An HTTP error with a JSON body.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// Maps a gRPC status onto the HTTP vocabulary.
#[must_use]
pub fn http_status(code: tonic::Code) -> StatusCode {
    match code {
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::InvalidArgument | tonic::Code::OutOfRange => StatusCode::BAD_REQUEST,
        tonic::Code::AlreadyExists => StatusCode::CONFLICT,
        tonic::Code::FailedPrecondition => StatusCode::PRECONDITION_FAILED,
        tonic::Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
        tonic::Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        tonic::Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn grpc(status: tonic::Status) -> ApiError {
    ApiError::new(http_status(status.code()), status.message().to_owned())
}

fn client(state: &AppState) -> Result<AuthClient, ApiError> {
    state
        .client
        .lock()
        .expect("daemon client poisoned")
        .clone()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "the daemon is not connected",
            )
        })
}

fn writable(state: &AppState) -> Result<(), ApiError> {
    if state.config.read_only {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "the gateway runs in read-only mode",
        ));
    }
    Ok(())
}

fn torrent_value(torrent: &Torrent) -> Value {
    Value::Object(sse::torrent_json(torrent))
}

/// `GET /api/status`
pub async fn status(
    State(state): State<Arc<AppState>>,
    _session: Session,
) -> Result<Response, ApiError> {
    let mut client = client(&state)?;
    let status = client
        .get_status(GetStatusRequest {})
        .await
        .map_err(grpc)?
        .into_inner();
    Ok(Json(json!({
        "torrents": status.torrent_count,
        "download_rate": status.download_rate,
        "upload_rate": status.upload_rate,
        "free_space_bytes": status.free_space_bytes,
        "states": {
            "downloading": status.downloading_count,
            "seeding": status.seeding_count,
            "queued": status.queued_count,
            "paused": status.paused_count,
            "error": status.error_count,
            "checking": status.checking_count,
            "moving": status.moving_count,
            "active": status.active_count,
        },
        "scheduler": {
            "enabled": status.scheduler_enabled,
            "level": status.scheduler_level,
            "next_unix": status.scheduler_next_unix,
        },
    }))
    .into_response())
}

/// Query of the torrent list.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub desc: bool,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

fn state_filter(name: &str) -> Result<i32, ApiError> {
    let state = match name.trim().to_ascii_lowercase().as_str() {
        "" | "all" => TorrentState::Unspecified,
        "queued" => TorrentState::Queued,
        "checking" => TorrentState::Checking,
        "downloading" | "down" => TorrentState::Downloading,
        "seeding" | "seed" => TorrentState::Seeding,
        "paused" => TorrentState::Paused,
        "error" => TorrentState::Error,
        "moving" => TorrentState::Moving,
        other => return Err(ApiError::bad(format!("unknown state {other:?}"))),
    };
    Ok(state as i32)
}

fn sort_field(name: &str) -> Result<i32, ApiError> {
    let field = match name.trim().to_ascii_lowercase().as_str() {
        "" | "id" => SortField::Id,
        "name" => SortField::Name,
        "progress" => SortField::Progress,
        "download" | "download_rate" | "down" => SortField::DownloadRate,
        "upload" | "upload_rate" | "up" => SortField::UploadRate,
        "queue" => SortField::Queue,
        "label" => SortField::Label,
        other => return Err(ApiError::bad(format!("unknown sort {other:?}"))),
    };
    Ok(field as i32)
}

/// `GET /api/torrents`
pub async fn list(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let mut client = client(&state)?;
    let limit = query.limit.unwrap_or(100).min(MAX_LIMIT);
    let response = client
        .list_torrents(ListTorrentsRequest {
            filter: query.filter,
            sort: sort_field(&query.sort)?,
            descending: query.desc,
            offset: query.offset.unwrap_or(0),
            limit,
            state_filter: state_filter(&query.state)?,
            label_filter: query.label,
        })
        .await
        .map_err(grpc)?
        .into_inner();
    let torrents: Vec<Value> = response.torrents.iter().map(torrent_value).collect();
    Ok(Json(json!({
        "torrents": torrents,
        "total": response.total,
        "limit": limit,
    }))
    .into_response())
}

/// `GET /api/torrents/{id}`
pub async fn detail(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    let mut client = client(&state)?;
    let response = client
        .get_torrent(GetTorrentRequest { id })
        .await
        .map_err(grpc)?
        .into_inner();
    let torrent = response
        .torrent
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("torrent {id} not found")))?;
    Ok(Json(torrent_value(&torrent)).into_response())
}

/// `GET /api/torrents/{id}/files`
pub async fn files(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    let mut client = client(&state)?;
    let response = client
        .get_torrent_files(GetTorrentFilesRequest { id })
        .await
        .map_err(grpc)?
        .into_inner();
    let files: Vec<Value> = response
        .files
        .iter()
        .map(|file| {
            json!({
                "path": file.path,
                "size": file.size,
                "progress": file.progress,
                "priority": file.priority,
            })
        })
        .collect();
    Ok(Json(json!({ "files": files })).into_response())
}

/// Body of an add.
#[derive(Debug, Default, Deserialize)]
pub struct AddBody {
    #[serde(default)]
    pub magnet: String,
    /// Base64 encoded `.torrent` bytes.
    #[serde(default)]
    pub metainfo: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub save_path: String,
    #[serde(default)]
    pub label: String,
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub paused: bool,
}

/// `POST /api/torrents`
pub async fn add(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Json(body): Json<AddBody>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let metainfo = if body.metainfo.trim().is_empty() {
        Vec::new()
    } else {
        BASE64
            .decode(body.metainfo.trim())
            .map_err(|error| ApiError::bad(format!("metainfo is not base64: {error}")))?
    };
    if metainfo.is_empty() && body.magnet.trim().is_empty() {
        return Err(ApiError::bad("either metainfo or magnet is required"));
    }
    let mut client = client(&state)?;
    let response = client
        .add_torrent(AddTorrentRequest {
            name: body.name,
            metainfo,
            save_path: body.save_path,
            total_bytes: body.total_bytes.unwrap_or(0),
            label: body.label,
            magnet: body.magnet,
        })
        .await
        .map_err(grpc)?
        .into_inner();
    if response.duplicate {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({ "id": response.id, "duplicate": true })),
        )
            .into_response());
    }
    if body.paused {
        client
            .pause_torrent(PauseTorrentRequest { id: response.id })
            .await
            .map_err(grpc)?;
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": response.id, "duplicate": false })),
    )
        .into_response())
}

/// `POST /api/torrents/{id}/pause`
pub async fn pause(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let mut client = client(&state)?;
    client
        .pause_torrent(PauseTorrentRequest { id })
        .await
        .map_err(grpc)?;
    Ok(Json(json!({ "id": id, "paused": true })).into_response())
}

/// `POST /api/torrents/{id}/resume`
pub async fn resume(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let mut client = client(&state)?;
    client
        .resume_torrent(ResumeTorrentRequest { id })
        .await
        .map_err(grpc)?;
    Ok(Json(json!({ "id": id, "paused": false })).into_response())
}

/// Query of a removal.
#[derive(Debug, Default, Deserialize)]
pub struct RemoveQuery {
    #[serde(default)]
    pub delete_files: bool,
}

/// `DELETE /api/torrents/{id}`
pub async fn remove(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
    Query(query): Query<RemoveQuery>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let mut client = client(&state)?;
    client
        .remove_torrent(RemoveTorrentRequest {
            id,
            delete_files: query.delete_files,
        })
        .await
        .map_err(grpc)?;
    Ok(Json(json!({ "id": id, "removed": true })).into_response())
}

/// Body of a label assignment.
#[derive(Debug, Deserialize)]
pub struct LabelBody {
    #[serde(default)]
    pub label: String,
}

/// `PUT /api/torrents/{id}/label`
pub async fn set_label(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(id): Path<u64>,
    Json(body): Json<LabelBody>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let mut client = client(&state)?;
    let response = client
        .set_torrent_label(SetTorrentLabelRequest {
            id,
            label: body.label,
        })
        .await
        .map_err(grpc)?
        .into_inner();
    Ok(Json(json!({ "id": id, "label": response.label })).into_response())
}

/// Body of a bulk action.
#[derive(Debug, Deserialize)]
pub struct BulkBody {
    pub action: String,
    pub ids: Vec<u64>,
}

/// `POST /api/torrents/bulk`
pub async fn bulk(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Json(body): Json<BulkBody>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    if body.ids.is_empty() {
        return Err(ApiError::bad("ids must not be empty"));
    }
    if body.ids.len() > MAX_BULK {
        return Err(ApiError::bad(format!("at most {MAX_BULK} ids per request")));
    }
    let action = body.action.trim().to_ascii_lowercase();
    if !matches!(action.as_str(), "pause" | "resume" | "remove") {
        return Err(ApiError::bad(format!(
            "unknown bulk action {:?}",
            body.action
        )));
    }
    let mut client = client(&state)?;
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for id in body.ids {
        let result = match action.as_str() {
            "pause" => client
                .pause_torrent(PauseTorrentRequest { id })
                .await
                .map(|_| ()),
            "resume" => client
                .resume_torrent(ResumeTorrentRequest { id })
                .await
                .map(|_| ()),
            _ => client
                .remove_torrent(RemoveTorrentRequest {
                    id,
                    delete_files: false,
                })
                .await
                .map(|_| ()),
        };
        match result {
            Ok(()) => ok.push(id),
            Err(error) => failed.push(json!({ "id": id, "error": error.message() })),
        }
    }
    Ok(Json(json!({ "ok": ok, "failed": failed })).into_response())
}

/// `GET /api/labels`
pub async fn labels(
    State(state): State<Arc<AppState>>,
    _session: Session,
) -> Result<Response, ApiError> {
    let mut client = client(&state)?;
    let response = client
        .list_labels(ListLabelsRequest {})
        .await
        .map_err(grpc)?
        .into_inner();
    let labels: Vec<Value> = response
        .labels
        .iter()
        .map(|entry| {
            let label = entry.label.clone().unwrap_or_default();
            json!({
                "name": label.name,
                "color": label.color,
                "move_completed_path": label.move_completed_path,
                "download_rate_limit": label.download_rate_limit,
                "upload_rate_limit": label.upload_rate_limit,
                "torrent_count": entry.torrent_count,
            })
        })
        .collect();
    Ok(Json(json!({ "labels": labels })).into_response())
}

/// Body of a label creation.
#[derive(Debug, Default, Deserialize)]
pub struct LabelCreate {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: String,
}

/// `POST /api/labels`
pub async fn create_label(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Json(body): Json<LabelCreate>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let name = body.name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return Err(ApiError::bad("label name must not be empty"));
    }
    if name.chars().count() > 64 {
        return Err(ApiError::bad("label name is too long"));
    }
    let color = if body.color.trim().is_empty() {
        "#2f81f7".to_owned()
    } else {
        body.color.trim().to_owned()
    };
    let mut client = client(&state)?;
    client
        .create_label(CreateLabelRequest {
            label: Some(Label {
                name: name.clone(),
                color,
                ..Label::default()
            }),
        })
        .await
        .map_err(grpc)?;
    Ok((StatusCode::CREATED, Json(json!({ "name": name }))).into_response())
}

/// `DELETE /api/labels/{name}`
pub async fn delete_label(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    writable(&state)?;
    let mut client = client(&state)?;
    let response = client
        .delete_label(DeleteLabelRequest { name: name.clone() })
        .await
        .map_err(grpc)?
        .into_inner();
    Ok(Json(json!({
        "name": name,
        "removed": response.removed,
        "cleared": response.cleared_assignments,
    }))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_codes_map_to_http() {
        assert_eq!(http_status(tonic::Code::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(
            http_status(tonic::Code::InvalidArgument),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            http_status(tonic::Code::AlreadyExists),
            StatusCode::CONFLICT
        );
        assert_eq!(
            http_status(tonic::Code::FailedPrecondition),
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            http_status(tonic::Code::PermissionDenied),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            http_status(tonic::Code::Unavailable),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn query_names_map_to_the_enums() {
        assert_eq!(
            state_filter("downloading").expect("state"),
            TorrentState::Downloading as i32
        );
        assert_eq!(
            state_filter("").expect("state"),
            TorrentState::Unspecified as i32
        );
        assert!(state_filter("nope").is_err());
        assert_eq!(sort_field("queue").expect("sort"), SortField::Queue as i32);
        assert!(sort_field("bogus").is_err());
    }
}
