//! qBittorrent Web API emulation (M8-T2).
//!
//! Sonarr/Radarr talk to a download client through qBittorrent's v2 API. This
//! module answers the handful of endpoints they actually use, backed by the
//! same engine, store and mirror as everything else.
//!
//! It is deliberately a *shim*: magnets are passed through to the engine
//! (which currently only accepts metainfo), and the response shapes mirror
//! qBittorrent's field names so an *arr client needs no special casing.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferrent_engine::{AddTorrent, EngineError, TorrentSource, TorrentState, TorrentStatus};
use serde_json::{Value, json};

use crate::config::schema::QbittorrentConfig;
use crate::persist::{self, PersistedTorrent};
use crate::rpc::AppState;

/// `sid` cookie handed out by /auth/login; one per daemon run.
pub const SESSION_COOKIE: &str = "SID";

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn form(&self) -> BTreeMap<String, String> {
        if self.is_multipart() {
            return self.multipart().0;
        }
        let text = String::from_utf8_lossy(&self.body);
        let mut fields = BTreeMap::new();
        for pair in text.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            fields.insert(decode(key), decode(value));
        }
        fields
    }

    fn is_multipart(&self) -> bool {
        self.header("content-type")
            .is_some_and(|value| value.starts_with("multipart/form-data"))
    }

    /// The form fields and the uploaded `.torrent` of a multipart body.
    /// *arr sends the category and the file in the very same request.
    fn multipart(&self) -> (BTreeMap<String, String>, Option<Vec<u8>>) {
        let mut fields = BTreeMap::new();
        let mut file = None;
        let Some(content_type) = self.header("content-type") else {
            return (fields, file);
        };
        let Some(boundary) = content_type.split("boundary=").nth(1) else {
            return (fields, file);
        };
        let marker = format!("--{}", boundary.trim_matches('"'));
        let haystack = String::from_utf8_lossy(&self.body).to_string();
        for part in haystack.split(&marker) {
            let Some((head, body)) = part.split_once("\r\n\r\n") else {
                continue;
            };
            let body = body.strip_suffix("\r\n").unwrap_or(body);
            let Some(name) = head
                .split("name=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
            else {
                continue;
            };
            if head.contains("filename=") {
                file = Some(body.as_bytes().to_vec());
            } else {
                fields.insert(name.to_owned(), body.to_owned());
            }
        }
        (fields, file)
    }

    /// The uploaded `.torrent` bytes of a multipart body, if any.
    fn torrent_file(&self) -> Option<Vec<u8>> {
        if self.is_multipart() {
            self.multipart().1
        } else {
            None
        }
    }
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                    out.push(byte);
                    index += 2;
                } else {
                    out.push(b'%');
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses one HTTP/1.1 request; enough for the *arr clients and curl.
#[must_use]
pub fn parse(raw: &[u8]) -> Option<Request> {
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body_start = split + 4;
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => {
            let mut fields = BTreeMap::new();
            for pair in query.split('&') {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                fields.insert(decode(key), decode(value));
            }
            (path.to_owned(), fields)
        }
        None => (target, BTreeMap::new()),
    };
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body = raw
        .get(body_start..body_start + length)
        .unwrap_or_default()
        .to_vec();
    Some(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
    pub set_cookie: Option<String>,
}

impl Response {
    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            body: value.to_string().into_bytes(),
            content_type: "application/json",
            set_cookie: None,
        }
    }

    fn text(body: &str) -> Self {
        Self {
            status: 200,
            body: body.as_bytes().to_vec(),
            content_type: "text/plain; charset=utf-8",
            set_cookie: None,
        }
    }

    fn fail(status: u16, message: &str) -> Self {
        Self {
            status,
            body: format!("{message}\n").into_bytes(),
            content_type: "text/plain; charset=utf-8",
            set_cookie: None,
        }
    }
}

fn state_name(state: TorrentState) -> &'static str {
    match state {
        TorrentState::Seeding => "uploading",
        TorrentState::Downloading => "downloading",
        TorrentState::Paused => "pausedDL",
        TorrentState::Checking => "checkingDL",
        TorrentState::Queued => "queuedDL",
        _ => "stalledUP",
    }
}

/// The `torrents/info` payload. Pure, so the contract is unit-testable.
#[must_use]
pub fn torrents_json(statuses: &[TorrentStatus], labels: &dyn Fn(u64) -> Option<String>) -> Value {
    let torrents: Vec<Value> = statuses
        .iter()
        .filter(|status| status.total_bytes > 0 || !status.name.is_empty())
        .map(|status| {
            let ratio = if status.downloaded_bytes == 0 {
                0.0
            } else {
                status.uploaded_bytes as f64 / status.downloaded_bytes as f64
            };
            json!({
                "hash": status.infohash,
                "name": status.name,
                "state": state_name(status.state.clone()),
                "progress": status.progress,
                "size": status.total_bytes,
                "total_size": status.total_bytes,
                "downloaded": status.downloaded_bytes,
                "uploaded": status.uploaded_bytes,
                "dlspeed": status.download_rate,
                "upspeed": status.upload_rate,
                "ratio": ratio,
                "eta": status.eta_seconds,
                "num_seeds": status.seeds,
                "num_leechs": status.peers,
                "category": labels(status.id).unwrap_or_default(),
                "save_path": status.save_path.display().to_string(),
                "completion_on": if status.progress >= 1.0 { 1 } else { 0 },
                "added_on": 0,
            })
        })
        .collect();
    Value::Array(torrents)
}

/// Handles one request against the live state.
pub async fn handle(
    request: &Request,
    state: &Arc<AppState>,
    config: &QbittorrentConfig,
) -> Response {
    let authenticated = config.username.is_empty()
        || request.header("cookie").is_some_and(|cookie| {
            cookie
                .split(';')
                .any(|part| part.trim() == format!("{SESSION_COOKIE}={}", session_id(config)))
        });

    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/api/v2/auth/login") => {
            let form = request.form();
            let user = form.get("username").cloned().unwrap_or_default();
            let password = form.get("password").cloned().unwrap_or_default();
            if user == config.username && password == config.password {
                let mut response = Response::text("Ok.");
                response.set_cookie = Some(format!(
                    "{SESSION_COOKIE}={}; path=/; HttpOnly; SameSite=Lax",
                    session_id(config)
                ));
                response
            } else {
                Response::fail(401, "Fails.")
            }
        }
        ("GET", "/api/v2/app/version") => Response::text("v4.6.5"),
        ("GET", "/api/v2/app/webapiVersion") => Response::text("2.9.3"),
        ("GET", "/api/v2/torrents/categories") => {
            let categories: Value = config
                .category_paths
                .iter()
                .map(|(name, path)| (name.clone(), json!({"name": name, "savePath": path})))
                .collect::<serde_json::Map<String, Value>>()
                .into();
            Response::json(&categories)
        }
        _ if !authenticated => Response::fail(403, "Forbidden"),
        ("GET", "/api/v2/torrents/info") => {
            let labels = state.labels.lock().expect("label state poisoned");
            let lookup = |id: u64| labels.label(id).map(str::to_owned);
            Response::json(&torrents_json(&state.mirror.all(), &lookup))
        }
        ("POST", "/api/v2/torrents/add") => add_torrent(request, state, config).await,
        ("POST", "/api/v2/torrents/pause") => set_paused(request, state, true).await,
        ("POST", "/api/v2/torrents/resume") => set_paused(request, state, false).await,
        ("POST", "/api/v2/torrents/delete") => delete(request, state).await,
        _ => Response::fail(404, "Not Found"),
    }
}

fn session_id(config: &QbittorrentConfig) -> String {
    // Stable per process: the cookie only marks "logged in to this daemon".
    format!("{:x}", config.password.len() ^ 0x5eed)
}

fn ids_for_hashes(request: &Request, state: &Arc<AppState>) -> Vec<u64> {
    let form = request.form();
    let hashes = form.get("hashes").map_or("all", String::as_str);
    let statuses = state.mirror.all();
    if hashes == "all" {
        return statuses.iter().map(|status| status.id).collect();
    }
    hashes
        .split('|')
        .filter_map(|hash| {
            statuses
                .iter()
                .find(|status| status.infohash.eq_ignore_ascii_case(hash))
                .map(|status| status.id)
        })
        .collect()
}

async fn set_paused(request: &Request, state: &Arc<AppState>, paused: bool) -> Response {
    for id in ids_for_hashes(request, state) {
        let immediate = {
            let mut control = state.control.lock().expect("pause control mutex poisoned");
            let changed = control.set_user(id, paused);
            (changed, control.is_paused(id))
        };
        if !immediate.0 {
            continue;
        }
        let engine = Arc::clone(&state.engine);
        let _ = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            if paused {
                engine.pause(id)
            } else {
                engine.resume(id)
            }
        })
        .await;
    }
    Response::text("Ok.")
}

async fn delete(request: &Request, state: &Arc<AppState>) -> Response {
    let form = request.form();
    let files = form.get("deleteFiles").is_some_and(|value| value == "true");
    for id in ids_for_hashes(request, state) {
        let engine = Arc::clone(&state.engine);
        let _ = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            engine.remove(id, files)
        })
        .await;
        {
            let mut control = state.control.lock().expect("pause control mutex poisoned");
            control.clear(id);
        }
        state
            .labels
            .lock()
            .expect("label state poisoned")
            .remove_torrent(id);
        let mut store = state.store.lock().expect("state store poisoned");
        let _ = persist::remove_torrent(&mut **store, id);
    }
    Response::text("Ok.")
}

async fn add_torrent(
    request: &Request,
    state: &Arc<AppState>,
    config: &QbittorrentConfig,
) -> Response {
    let form = request.form();
    let Some(metainfo) = request.torrent_file() else {
        let urls = form.get("urls").cloned().unwrap_or_default();
        if urls.starts_with("magnet:") {
            return Response::fail(
                415,
                "Fails. Magnet links need the engine's metadata feature; send the .torrent file.",
            );
        }
        return Response::fail(400, "Fails. No torrent file in the request.");
    };
    let summary = match crate::autoadd::Summary::parse(&metainfo) {
        Ok(summary) => summary,
        Err(error) => return Response::fail(400, &format!("Fails. {error}")),
    };
    let category = form.get("category").cloned().unwrap_or_default();
    let requested = form
        .get("savepath")
        .filter(|path| !path.is_empty())
        .cloned()
        .or_else(|| config.category_paths.get(&category).cloned())
        .unwrap_or_default();
    let config_resolved = match state.config.resolved() {
        Ok(config) => config,
        Err(error) => return Response::fail(500, &error.to_string()),
    };
    // A configured category path is taken as-is: it is the operator's
    // decision, not something the label policy should re-derive.
    let save_path = if requested.is_empty() {
        match crate::diskguard::paths::download_path(&config_resolved, "", &category) {
            Ok(path) => path,
            Err(error) => return Response::fail(409, &error),
        }
    } else {
        std::path::PathBuf::from(requested)
    };
    if let crate::diskguard::paths::SpaceGate::Reject(message) =
        crate::diskguard::paths::space_gate(&config_resolved, &save_path, summary.total_bytes)
    {
        return Response::fail(409, &message);
    }

    let spec = AddTorrent {
        name: summary.name.clone(),
        total_bytes: summary.total_bytes,
        source: TorrentSource::Metainfo(metainfo.clone()),
        save_path: save_path.clone(),
        resume_data: None,
    };
    let engine = Arc::clone(&state.engine);
    let add = tokio::task::spawn_blocking(move || {
        let mut engine = engine.lock().expect("engine mutex poisoned");
        engine.add_torrent(spec)
    })
    .await;
    let id = match add {
        Ok(Ok(id)) => id,
        Ok(Err(EngineError::AlreadyExists { .. })) => return Response::text("Ok."),
        Ok(Err(error)) => return Response::fail(409, &error.to_string()),
        Err(error) => return Response::fail(500, &error.to_string()),
    };
    let persisted = PersistedTorrent {
        id,
        name: summary.name.clone(),
        save_path,
        metainfo,
        resume_data: None,
        paused_by_user: false,
        label: None,
    };
    {
        let mut store = state.store.lock().expect("state store poisoned");
        if let Err(error) = persist::save_torrent(&mut **store, &persisted) {
            return Response::fail(500, &error.to_string());
        }
    }
    if !category.is_empty() {
        // *arr sends the category on every add, so the emulation owns the
        // label too: create it when the operator configured the category.
        let mut labels = state.labels.lock().expect("label state poisoned");
        if labels.get(&category).is_none() {
            let _ = labels.upsert(crate::labels::Label {
                name: category.clone(),
                ..crate::labels::Label::default()
            });
        }
        let _ = labels.assign(id, Some(category));
    }
    Response::text("Ok.")
}

/// Serves the emulation until the process ends.
pub async fn serve(state: Arc<AppState>) {
    let config = match state.config.resolved() {
        Ok(config) => config.compat.qbittorrent,
        Err(error) => {
            eprintln!("qbt: {error}");
            return;
        }
    };
    if !config.enabled {
        return;
    }
    let listener = match tokio::net::TcpListener::bind(&config.listen).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("qbt: cannot bind {}: {error}", config.listen);
            return;
        }
    };
    println!("qbt emulation listening on http://{}/api/v2", config.listen);
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        let config = config.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buffer = vec![0u8; 64 * 1024];
            let Ok(read) = socket.read(&mut buffer).await else {
                return;
            };
            let Some(request) = parse(&buffer[..read]) else {
                return;
            };
            let response = handle(&request, &state, &config).await;
            let mut head = format!(
                "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n",
                response.status,
                if response.status == 200 {
                    "OK"
                } else {
                    "Error"
                },
                response.content_type,
                response.body.len()
            );
            if let Some(cookie) = response.set_cookie {
                head.push_str(&format!("set-cookie: {cookie}\r\n"));
            }
            head.push_str("\r\n");
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&response.body).await;
            let _ = socket.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn status(id: u64, name: &str, state: TorrentState) -> TorrentStatus {
        TorrentStatus {
            id,
            infohash: format!("{id:040x}"),
            name: name.to_owned(),
            save_path: PathBuf::from("/data"),
            total_bytes: 1000,
            state,
            progress: 1.0,
            download_rate: 1,
            upload_rate: 2,
            uploaded_bytes: 3000,
            downloaded_bytes: 1000,
            seeds: 1,
            peers: 2,
            eta_seconds: 0,
            queue_position: -1,
        }
    }

    #[test]
    fn parses_a_request_with_query_and_body() {
        let body = "hashes=all";
        let raw = format!(
            "POST /api/v2/torrents/add?x=1 HTTP/1.1\r\nContent-Length: {}\r\nCookie: SID=1\r\n\r\n{body}",
            body.len()
        );
        let request = parse(raw.as_bytes()).expect("parse");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/v2/torrents/add");
        assert_eq!(request.query.get("x").map(String::as_str), Some("1"));
        assert_eq!(request.header("cookie"), Some("SID=1"));
        assert_eq!(request.body, b"hashes=all");
    }

    #[test]
    fn extracts_a_multipart_torrent() {
        let body = concat!(
            "--X\r\nContent-Disposition: form-data; name=\"torrents\"; filename=\"a.torrent\"\r\n",
            "Content-Type: application/x-bittorrent\r\n\r\nd4:infod4:name1:a6:lengthi1eee\r\n--X--\r\n"
        );
        let raw = format!(
            "POST /api/v2/torrents/add HTTP/1.1\r\ncontent-type: multipart/form-data; boundary=X\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let request = parse(raw.as_bytes()).expect("parse");
        assert_eq!(
            request.torrent_file().as_deref(),
            Some(&b"d4:infod4:name1:a6:lengthi1eee"[..])
        );
    }

    #[test]
    fn multipart_fields_arrive_next_to_the_file() {
        let body = concat!(
            "--X\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\ntv\r\n",
            "--X\r\nContent-Disposition: form-data; name=\"torrents\"; filename=\"a.torrent\"\r\n",
            "Content-Type: application/x-bittorrent\r\n\r\nd4:infoe\r\n--X--\r\n"
        );
        let raw = format!(
            "POST /api/v2/torrents/add HTTP/1.1\r\ncontent-type: multipart/form-data; boundary=X\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let request = parse(raw.as_bytes()).expect("parse");
        assert_eq!(
            request.form().get("category").map(String::as_str),
            Some("tv")
        );
        assert_eq!(request.torrent_file().as_deref(), Some(&b"d4:infoe"[..]));
    }

    #[test]
    fn info_uses_qbittorrent_field_names() {
        let statuses = [status(1, "show.mkv", TorrentState::Seeding)];
        let value = torrents_json(&statuses, &|id| (id == 1).then(|| "tv".to_owned()));
        let torrent = &value[0];
        assert_eq!(torrent["name"], "show.mkv");
        assert_eq!(torrent["state"], "uploading");
        assert_eq!(torrent["progress"], 1.0);
        assert_eq!(torrent["size"], 1000);
        assert_eq!(torrent["ratio"], 3.0);
        assert_eq!(torrent["category"], "tv");
    }

    #[test]
    fn a_paused_torrent_reports_the_qbittorrent_state() {
        let statuses = [status(1, "x", TorrentState::Paused)];
        assert_eq!(torrents_json(&statuses, &|_| None)[0]["state"], "pausedDL");
    }
}
