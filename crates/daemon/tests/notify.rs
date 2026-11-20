#![cfg(unix)]

//! Notification webhook tests against a local fake HTTP server.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::events::{EventKind, NewEvent};
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::notify::{self, Delivery};
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::MockEngine;

/// Reads one HTTP request (headers plus the Content-Length body).
fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buffer);
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if buffer.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Serves exactly one request with `status` and returns the captured request.
fn fake_server(status: u16) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);
        let response =
            format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream.write_all(response.as_bytes()).expect("write");
        request
    });
    (format!("http://{address}/hook"), handle)
}

fn delivery(url: String, secret: &str) -> Delivery {
    Delivery {
        seq: 1,
        event_seq: 2,
        event: EventKind::Finished,
        torrent_id: 7,
        name: "movie.mkv".to_owned(),
        infohash: "ab".repeat(20),
        label: "movies".to_owned(),
        url,
        secret: secret.to_owned(),
        timeout_ms: 2000,
        attempt: 0,
        next_at: 0,
        dead: false,
        last_status: None,
        last_error: None,
        created_at: 1_700_000_000,
    }
}

#[test]
fn webhook_posts_json_with_headers_and_signature() {
    let (url, handle) = fake_server(200);
    let outcome = notify::send_http(&delivery(url, "s3cret"));
    assert_eq!(outcome.status, Some(200), "{outcome:?}");
    assert!(outcome.error.is_none());

    let request = handle.join().expect("server");
    let lower = request.to_ascii_lowercase();
    assert!(lower.contains("x-ferrent-event: finished"), "{request}");
    assert!(lower.contains("x-ferrent-delivery: 1"), "{request}");
    assert!(lower.contains("x-ferrent-signature: sha256="), "{request}");
    assert!(
        lower.contains("content-type: application/json"),
        "{request}"
    );
    assert!(request.contains("\"name\":\"movie.mkv\""), "{request}");
    assert!(request.contains("\"label\":\"movies\""), "{request}");
}

#[test]
fn a_five_hundred_is_retryable_and_a_four_hundred_is_not() {
    let (url, handle) = fake_server(500);
    let outcome = notify::send_http(&delivery(url, ""));
    assert_eq!(outcome.status, Some(500));
    assert!(outcome.retryable());
    handle.join().expect("server");

    let (url, handle) = fake_server(400);
    let outcome = notify::send_http(&delivery(url, ""));
    assert_eq!(outcome.status, Some(400));
    assert!(!outcome.retryable());
    handle.join().expect("server");
}

#[tokio::test]
async fn a_published_event_reaches_the_webhook_and_acks() {
    let (url, handle) = fake_server(200);
    let config = ConfigStore::default();
    config
        .apply_toml(
            &format!(
                "[notify]\nenabled = true\n\n[[notify.webhook]]\nurl = {url:?}\nsecret = \"s3cret\"\nevents = [\"finished\"]\n"
            ),
            0,
        )
        .expect("config");
    let (send, _) = tokio::sync::broadcast::channel(8);
    let state = Arc::new(AppState::new(
        Box::new(MockEngine::new()),
        Arc::new(StateMirror::new()),
        send,
        Box::new(MemoryStore::new()),
        config,
        TorrentControl::default(),
        LabelState::default(),
        "token",
        std::env::temp_dir(),
        std::env::temp_dir(),
    ));

    state
        .publish_event(NewEvent::new(EventKind::Finished, 7, "movie.mkv"))
        .await
        .expect("publish");
    let status = state.notification_status().await.expect("status");
    assert_eq!(status.pending, 1, "one delivery is queued");

    let report = state.notify_tick(2_000_000_000).await.expect("tick");
    assert_eq!(report.sent, 1, "{report:?}");
    let status = state.notification_status().await.expect("status");
    assert_eq!(status.pending, 0);
    assert_eq!(status.dead, 0);

    let request = handle.join().expect("server");
    assert!(request.contains("\"id\":7"), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-ferrent-signature: sha256=")
    );
}

#[tokio::test]
async fn an_event_without_matching_channels_queues_nothing() {
    let config = ConfigStore::default();
    config
        .apply_toml(
            "[[notify.webhook]]\nurl = \"https://example.test/hook\"\nevents = [\"error\"]\n",
            0,
        )
        .expect("config");
    let (send, _) = tokio::sync::broadcast::channel(8);
    let state = Arc::new(AppState::new(
        Box::new(MockEngine::new()),
        Arc::new(StateMirror::new()),
        send,
        Box::new(MemoryStore::new()),
        config,
        TorrentControl::default(),
        LabelState::default(),
        "token",
        std::env::temp_dir(),
        std::env::temp_dir(),
    ));
    state
        .publish_event(NewEvent::new(EventKind::Finished, 1, "x"))
        .await
        .expect("publish");
    assert_eq!(
        state.notification_status().await.expect("status").pending,
        0
    );
}
