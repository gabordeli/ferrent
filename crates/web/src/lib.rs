//! `ferrent-web`: a small gateway in front of `ferrentd`.
//!
//! The daemon speaks TLS gRPC with a bearer token; browsers cannot. This crate
//! keeps one daemon connection, exposes a cookie-authenticated HTTP surface and
//! bridges the daemon's `Watch` stream as coalesced SSE. TLS is terminated by
//! the reverse proxy in front of it (nginx), so the listener stays on loopback.

pub mod api;
pub mod auth;
pub mod config;
pub mod sse;
pub mod tls;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use ferrent_protocol::client::AuthClient;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;

pub use config::{DaemonConfig, TlsConfig, WebConfig};

/// Shared state of the gateway.
pub struct AppState {
    pub config: WebConfig,
    pub sessions: auth::Sessions,
    pub limiter: auth::LoginLimiter,
    /// Coalesced deltas fanned out to every SSE client.
    pub hub: broadcast::Sender<String>,
    /// The daemon client, once connected.
    pub client: Mutex<Option<AuthClient>>,
    /// Whether the daemon connection is up (for `/health`).
    pub daemon_up: AtomicBool,
    /// Whether the session cookie needs `Secure` (the listener is HTTPS).
    pub secure: bool,
}

impl AppState {
    /// Builds the state from a validated configuration.
    #[must_use]
    pub fn new(config: WebConfig) -> Arc<Self> {
        let (hub, _) = broadcast::channel(64);
        Arc::new(Self {
            sessions: auth::Sessions::new(config.session_ttl()),
            limiter: auth::LoginLimiter::new(config.login_per_minute),
            hub,
            client: Mutex::new(None),
            daemon_up: AtomicBool::new(false),
            secure: config.tls.is_some(),
            config,
        })
    }
}

/// Marker for an authenticated request.
pub struct Session;

impl FromRequestParts<Arc<AppState>> for Session {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok());
        let token = header.and_then(|header| auth::cookie(header, auth::COOKIE));
        match token {
            Some(token) if state.sessions.valid(token, Instant::now()) => Ok(Self),
            _ => Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "login required" })),
            )
                .into_response()),
        }
    }
}

/// The HTTP surface of the gateway.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        .route("/api/events", get(events))
        .route("/api/status", get(api::status))
        .route("/api/torrents", get(api::list).post(api::add))
        .route("/api/torrents/bulk", post(api::bulk))
        .route("/api/torrents/{id}", get(api::detail).delete(api::remove))
        .route("/api/torrents/{id}/files", get(api::files))
        .route("/api/torrents/{id}/pause", post(api::pause))
        .route("/api/torrents/{id}/resume", post(api::resume))
        .route("/api/torrents/{id}/label", put(api::set_label))
        .route("/api/labels", get(api::labels).post(api::create_label))
        .route(
            "/api/labels/{name}",
            axum::routing::delete(api::delete_label),
        )
        .fallback(get(spa))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    password: String,
}

/// The client key for the login limiter: the proxy header, or "direct".
fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("direct")
        .to_owned()
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let up = state.daemon_up.load(Ordering::Relaxed);
    let body = Json(json!({
        "status": if up { "ok" } else { "degraded" },
        "daemon": up,
        "read_only": state.config.read_only,
    }));
    (
        if up {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        body,
    )
        .into_response()
}

async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Response {
    let now = Instant::now();
    if !state.limiter.allow(&client_key(&headers), now) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "too many login attempts" })),
        )
            .into_response();
    }
    if !auth::constant_time_eq(&request.password, &state.config.password) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "wrong password" })),
        )
            .into_response();
    }
    let token = state.sessions.issue(now);
    let cookie = format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}{}",
        auth::COOKIE,
        state.config.session_ttl_secs,
        if state.secure { "; Secure" } else { "" }
    );
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "read_only": state.config.read_only, "ttl": state.config.session_ttl_secs })),
    )
        .into_response()
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(token) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| auth::cookie(header, auth::COOKIE))
    {
        state.sessions.revoke(token);
    }
    let cookie = format!(
        "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        auth::COOKIE,
        if state.secure { "; Secure" } else { "" }
    );
    (StatusCode::OK, [(header::SET_COOKIE, cookie)]).into_response()
}

async fn session(State(state): State<Arc<AppState>>, _session: Session) -> Response {
    Json(json!({
        "read_only": state.config.read_only,
        "daemon": state.daemon_up.load(Ordering::Relaxed),
        "sessions": state.sessions.len(),
    }))
    .into_response()
}

async fn events(State(state): State<Arc<AppState>>, _session: Session) -> Response {
    sse::stream(Arc::clone(&state)).into_response()
}

/// Serves the built SPA from `spa_dir`, falling back to `index.html` for
/// client-side routes. Without `spa_dir` a placeholder explains the state.
async fn spa(State(state): State<Arc<AppState>>, uri: axum::http::Uri) -> Response {
    let Some(dir) = state.config.spa_dir.clone() else {
        return axum::response::Html(PLACEHOLDER).into_response();
    };
    let requested = uri.path().trim_start_matches('/');
    let candidate = if requested.is_empty() {
        dir.join("index.html")
    } else {
        dir.join(requested)
    };
    if candidate
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = if candidate.is_file() {
        candidate
    } else {
        dir.join("index.html")
    };
    match std::fs::read(&file) {
        Ok(bytes) => {
            let mut response =
                ([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response();
            if file
                .extension()
                .is_some_and(|extension| extension == "html")
            {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "no-cache".parse().expect("header"));
            }
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

const PLACEHOLDER: &str = "<!doctype html><meta charset=\"utf-8\"><title>Ferrent</title>\
<body style=\"background:#0b1220;color:#c9d4e3;font-family:system-ui;padding:2rem\">\
<h1>Ferrent</h1><p>The web UI is not built yet; the API is under <code>/api</code>.</p>";

fn content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json" | "webmanifest") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Reads `X-Forwarded-For` on a request (tests and the proxy set it).
#[must_use]
pub fn forwarded(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// A convenience for the binary: build the state and the router.
pub fn app(config: WebConfig) -> (Arc<AppState>, Router) {
    let state = AppState::new(config);
    let router = router(Arc::clone(&state));
    (state, router)
}

/// Reads the request body limit used by axum's JSON extractor default.
pub const BODY_LIMIT: usize = 1024 * 1024;
