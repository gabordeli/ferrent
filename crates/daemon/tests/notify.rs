#![cfg(unix)]

//! Notification channel tests against a local fake HTTP server.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::events::{EventKind, NewEvent};
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::notify::{
    self, Delivery, DigestPolicy, NotifyRules, SendOutcome, TelegramRule, WebhookRule,
};
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::MockEngine;

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

/// Serves one request with `status` and `body`; returns the captured request.
fn fake_server(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("write");
        request
    });
    (format!("http://{address}"), handle)
}

fn webhook_delivery(base: &str, secret: &str) -> Delivery {
    Delivery {
        seq: 1,
        event_seq: 2,
        event: EventKind::Finished,
        torrent_id: 7,
        name: "movie.mkv".to_owned(),
        infohash: "ab".repeat(20),
        label: "movies".to_owned(),
        url: format!("{base}/hook"),
        secret: secret.to_owned(),
        telegram: None,
        timeout_ms: 2000,
        attempt: 0,
        next_at: 0,
        dead: false,
        last_status: None,
        last_error: None,
        created_at: 1_700_000_000,
    }
}

fn telegram_delivery(base: &str) -> Delivery {
    Delivery {
        seq: 1,
        event_seq: 2,
        event: EventKind::Finished,
        torrent_id: 7,
        name: "movie.mkv".to_owned(),
        infohash: String::new(),
        label: String::new(),
        url: String::new(),
        secret: String::new(),
        telegram: Some(notify::TelegramTarget {
            api_base: base.to_owned(),
            bot_token: "123:abc".to_owned(),
            chat_id: "-100".to_owned(),
            message_thread_id: "5".to_owned(),
        }),
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
    let (base, handle) = fake_server(200, "");
    let outcome = notify::send(&webhook_delivery(&base, "s3cret"), None);
    assert_eq!(outcome.status, Some(200), "{outcome:?}");

    let request = handle.join().expect("server");
    let lower = request.to_ascii_lowercase();
    assert!(lower.contains("x-ferrent-event: finished"), "{request}");
    assert!(lower.contains("x-ferrent-delivery: 1"), "{request}");
    assert!(lower.contains("x-ferrent-signature: sha256="), "{request}");
    assert!(request.contains("\"name\":\"movie.mkv\""), "{request}");
}

#[test]
fn telegram_posts_send_message_with_thread_and_html() {
    let (base, handle) = fake_server(200, "");
    let outcome = notify::send(&telegram_delivery(&base), None);
    assert_eq!(outcome.status, Some(200), "{outcome:?}");

    let request = handle.join().expect("server");
    assert!(
        request.starts_with("POST /bot123:abc/sendMessage"),
        "{request}"
    );
    assert!(request.contains("\"chat_id\":\"-100\""), "{request}");
    assert!(request.contains("\"message_thread_id\":\"5\""), "{request}");
    assert!(request.contains("\"parse_mode\":\"HTML\""), "{request}");
    assert!(request.contains("movie.mkv"), "{request}");
}

#[test]
fn telegram_429_reports_the_retry_after_hint() {
    let (base, handle) = fake_server(429, r#"{"ok":false,"parameters":{"retry_after":7}}"#);
    let outcome = notify::send(&telegram_delivery(&base), None);
    assert_eq!(outcome.status, Some(429));
    assert_eq!(outcome.retry_after, Some(7));
    assert!(outcome.retryable());
    handle.join().expect("server");
}

#[test]
fn a_five_hundred_is_retryable_and_a_four_hundred_is_not() {
    let (base, handle) = fake_server(500, "");
    let outcome = notify::send(&webhook_delivery(&base, ""), None);
    assert!(outcome.retryable());
    handle.join().expect("server");

    let (base, handle) = fake_server(400, "");
    let outcome = notify::send(&webhook_delivery(&base, ""), None);
    assert!(!outcome.retryable());
    handle.join().expect("server");
}

#[test]
fn quiet_hours_hold_telegram_deliveries() {
    let mut store = MemoryStore::new();
    let webhooks: Vec<WebhookRule> = Vec::new();
    let telegram = vec![TelegramRule {
        bot_token: "t".to_owned(),
        chat_id: "1".to_owned(),
        api_base: "http://127.0.0.1:9".to_owned(),
        ..TelegramRule::default()
    }];
    let event = ferrent_daemon::events::HookEvent {
        seq: 1,
        kind: EventKind::Finished,
        torrent_id: 1,
        name: "x".to_owned(),
        save_path: String::new(),
        infohash: String::new(),
        error: None,
        at_unix: 1,
    };
    for seq in 1..=10 {
        let mut event = event.clone();
        event.seq = seq;
        notify::enqueue_at(
            &mut store,
            &event,
            "",
            NotifyRules {
                webhooks: &webhooks,
                telegram: &telegram,
                label_filter: "",
            },
            0,
        )
        .expect("queue");
    }
    let policy = DigestPolicy {
        events: 10,
        window_secs: 30,
        quiet: Some((23 * 60, 7 * 60)),
        local_minute: 23 * 60 + 30,
    };
    let report =
        notify::process(&mut store, 0, policy, &mut |_, _| SendOutcome::ok(200)).expect("process");
    assert_eq!(report.sent, 0, "quiet hours hold the digest");
    let deliveries = notify::deliveries(&store).expect("deliveries");
    assert_eq!(deliveries.len(), 10);
    assert!(deliveries.iter().all(|delivery| delivery.next_at > 0));
}

#[tokio::test]
async fn a_published_event_reaches_the_webhook_and_acks() {
    let (base, handle) = fake_server(200, "");
    let config = ConfigStore::default();
    config
        .apply_toml(
            &format!(
                "[notify]\nenabled = true\ndigest_events = 0\n\n[[notify.webhook]]\nurl = \"{base}/hook\"\nsecret = \"s3cret\"\nevents = [\"finished\"]\n"
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
    assert_eq!(
        state.notification_status().await.expect("status").pending,
        1
    );

    let report = state.notify_tick(2_000_000_000).await.expect("tick");
    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(
        state.notification_status().await.expect("status").pending,
        0
    );

    let request = handle.join().expect("server");
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
