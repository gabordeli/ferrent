//! REST surface against a real in-process daemon (TLS, MockEngine).

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{MockEngine, TorrentEngine};
use ferrent_web::{DaemonConfig, PushConfig, WebConfig, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower::ServiceExt;

struct Daemon {
    addr: SocketAddr,
    ca: PathBuf,
    dir: PathBuf,
    server: tokio::task::JoinHandle<()>,
    mirror: tokio::task::JoinHandle<()>,
}

async fn start_daemon() -> Daemon {
    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
    let mock = MockEngine::new();
    let events = mock.subscribe();
    let engine: Box<dyn TorrentEngine> = Box::new(mock);
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(64);
    let mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let state = Arc::new(AppState::new(
        engine,
        mirror,
        watch,
        Box::new(MemoryStore::new()),
        ConfigStore::default(),
        TorrentControl::default(),
        LabelState::default(),
        "secret",
        std::env::temp_dir(),
    ));
    let tls = TlsMaterial::generate().expect("tls");
    let dir = std::env::temp_dir().join(format!(
        "ferrent-web-api-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let ca = dir.join("ca.pem");
    std::fs::write(&ca, &tls.cert_pem).expect("write CA");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server_tls = tls.clone();
    let server = tokio::spawn(async move {
        serve(listener, state, &server_tls).await.expect("serve");
    });
    Daemon {
        addr,
        ca,
        dir,
        server,
        mirror: mirror_task,
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.server.abort();
        self.mirror.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Web {
    router: Router,
    cookie: String,
    state: Arc<ferrent_web::AppState>,
}

async fn start_web(daemon: &Daemon, read_only: bool) -> Web {
    let config = WebConfig {
        password: "test-pass".to_owned(),
        read_only,
        daemon: DaemonConfig {
            url: format!("https://localhost:{}", daemon.addr.port()),
            ca: daemon.ca.clone(),
            token: "secret".to_owned(),
            fingerprint: None,
        },
        ..WebConfig::default()
    };
    let (state, router) = app(config);
    let client = ferrent_protocol::client::connect(
        &format!("https://localhost:{}", daemon.addr.port()),
        &std::fs::read_to_string(&daemon.ca).expect("CA"),
        "secret",
    )
    .await
    .expect("daemon client");
    *state.client.lock().expect("client") = Some(client);
    let cookie = format!(
        "ferrent_session={}",
        state.sessions.issue(std::time::Instant::now())
    );
    Web {
        router,
        cookie,
        state,
    }
}

async fn call(web: &Web, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &web.cookie);
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = web
        .router
        .clone()
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn wait_for_total(web: &Web, total: u64) {
    for _ in 0..40 {
        let (status, body) = call(web, "GET", "/api/torrents", None).await;
        if status == StatusCode::OK && body["total"].as_u64() == Some(total) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the mirror did not reach {total} torrents");
}

#[tokio::test]
async fn the_rest_surface_mirrors_the_daemon() {
    let daemon = start_daemon().await;
    let web = start_web(&daemon, false).await;

    let (status, body) = call(&web, "GET", "/api/status", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Add a magnet, then list it.
    let (status, added) = call(
        &web,
        "POST",
        "/api/torrents",
        Some(json!({ "magnet": "magnet:?xt=urn:btih:00112233445566778899aabbccddeeff00112233", "name": "first.mkv" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    let id = added["id"].as_u64().expect("id");
    wait_for_total(&web, 1).await;

    let (status, list) = call(&web, "GET", "/api/torrents?sort=name&limit=10", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["total"], 1);
    assert_eq!(list["torrents"][0]["id"], id);

    let (status, detail) = call(&web, "GET", &format!("/api/torrents/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["name"], "first.mkv");

    // The same info hash is a conflict carrying the existing id.
    let (status, duplicate) = call(
        &web,
        "POST",
        "/api/torrents",
        Some(json!({ "magnet": "magnet:?xt=urn:btih:00112233445566778899aabbccddeeff00112233", "name": "first.mkv" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    assert_eq!(duplicate["id"], id);

    // Pause shows up in the detail after the mirror publishes.
    let (status, _) = call(&web, "POST", &format!("/api/torrents/{id}/pause"), None).await;
    assert_eq!(status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (_, detail) = call(&web, "GET", &format!("/api/torrents/{id}"), None).await;
    assert_eq!(detail["state"], 5, "paused: {detail}");

    // Files are empty for a metadata-less magnet, but the shape holds.
    let (status, files) = call(&web, "GET", &format!("/api/torrents/{id}/files"), None).await;
    assert_eq!(status, StatusCode::OK, "{files}");
    assert!(files["files"].is_array());

    // Labels come from the daemon list.
    let (status, labels) = call(&web, "GET", "/api/labels", None).await;
    assert_eq!(status, StatusCode::OK, "{labels}");
    assert!(labels["labels"].is_array());

    // Unknown ids and bad queries are 404/400.
    let (status, _) = call(&web, "GET", "/api/torrents/999999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&web, "GET", "/api/torrents?state=nope", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(&web, "DELETE", &format!("/api/torrents/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    wait_for_total(&web, 0).await;
    assert_eq!(web.state.sessions.len(), 1);
}

#[tokio::test]
async fn bulk_actions_report_partial_results() {
    let daemon = start_daemon().await;
    let web = start_web(&daemon, false).await;
    let mut ids = Vec::new();
    for index in 0..3 {
        let (status, body) = call(
            &web,
            "POST",
            "/api/torrents",
            Some(json!({
                "magnet": format!("magnet:?xt=urn:btih:{index:040x}"),
                "name": format!("t{index}.mkv"),
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        ids.push(body["id"].as_u64().expect("id"));
    }
    wait_for_total(&web, 3).await;

    let (status, body) = call(
        &web,
        "POST",
        "/api/torrents/bulk",
        Some(json!({ "action": "pause", "ids": ids })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"].as_array().expect("ok").len(), 3);
    assert!(body["failed"].as_array().expect("failed").is_empty());

    let (status, body) = call(
        &web,
        "POST",
        "/api/torrents/bulk",
        Some(json!({ "action": "remove", "ids": ids })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    wait_for_total(&web, 0).await;

    let (status, _) = call(
        &web,
        "POST",
        "/api/torrents/bulk",
        Some(json!({ "action": "pause", "ids": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_read_only_gateway_refuses_mutations() {
    let daemon = start_daemon().await;
    let web = start_web(&daemon, true).await;

    let (status, body) = call(
        &web,
        "POST",
        "/api/torrents",
        Some(json!({ "magnet": "magnet:?xt=urn:btih:aa" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body["error"].as_str().expect("error").contains("read-only"));

    let (status, _) = call(&web, "GET", "/api/torrents", None).await;
    assert_eq!(status, StatusCode::OK, "reads stay open");
}

fn web_from_config(config: WebConfig) -> Web {
    let (state, router) = app(config);
    let cookie = format!(
        "ferrent_session={}",
        state.sessions.issue(std::time::Instant::now())
    );
    Web {
        router,
        cookie,
        state,
    }
}

fn push_web(dir: &std::path::Path, enabled: bool) -> Web {
    let config = WebConfig {
        password: "test-pass".to_owned(),
        push: PushConfig {
            enabled,
            subject: "mailto:test@example.com".to_owned(),
            private_key: dir.join("vapid.pem"),
            subscriptions: dir.join("subs.json"),
        },
        daemon: DaemonConfig {
            url: "https://localhost:50129".to_owned(),
            ca: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
            token: "token".to_owned(),
            fingerprint: None,
        },
        ..WebConfig::default()
    };
    web_from_config(config)
}

#[tokio::test]
async fn push_subscriptions_roundtrip_and_a_disabled_gateway() {
    let dir = std::env::temp_dir().join(format!("ferrent-web-push-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");

    let web = push_web(&dir, true);
    let (status, body) = call(&web, "GET", "/api/push/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    let public_key = body["public_key"].as_str().expect("public key");
    assert!(!public_key.is_empty());

    let request = Request::builder()
        .method("GET")
        .uri("/api/push/config")
        .body(Body::empty())
        .expect("request");
    let anonymous = web.router.clone().oneshot(request).await.expect("response");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([4_u8; 65]);
    let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1_u8; 16]);
    let subscription = json!({
        "endpoint": "https://push.example/1",
        "keys": { "p256dh": &p256dh, "auth": &auth },
    });

    let (status, body) = call(
        &web,
        "POST",
        "/api/push/subscribe",
        Some(json!({
            "endpoint": "http://plain.example/1",
            "keys": { "p256dh": &p256dh, "auth": &auth },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = call(
        &web,
        "POST",
        "/api/push/subscribe",
        Some(subscription.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 1);

    let (status, body) = call(&web, "GET", "/api/push/subscriptions", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body["subscriptions"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["host"], "push.example");
    let id = rows[0]["id"].as_str().expect("id").to_owned();

    let (status, body) = call(
        &web,
        "POST",
        "/api/push/test",
        Some(json!({ "endpoint": "https://push.example/1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["queued"], true);

    let (status, body) = call(
        &web,
        "DELETE",
        &format!("/api/push/subscriptions/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["unsubscribed"], true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("subs.json"))
            .expect("store")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the client keys stay private");
    }

    let off = dir.join("off");
    let disabled = push_web(&off, false);
    let (status, body) = call(&disabled, "GET", "/api/push/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], false);

    let (status, _) = call(&disabled, "POST", "/api/push/subscribe", Some(subscription)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(dir);
}
