//! Notification router: domain events → rules → durable outbox → webhook.
//!
//! Each (event, channel) pair becomes one outbox record before delivery, so a
//! restart resumes instead of duplicating. Failures retry with exponential
//! backoff; after [`MAX_ATTEMPTS`] the record is marked `dead` and kept for the
//! status view. Secrets are used for the HMAC header and never logged.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::events::{EventKind, HookEvent};
use crate::store::{StateStore, StoreError, StoreOp};

/// Key prefix of one outbox delivery.
pub const DELIVERY_PREFIX: &str = "notify:delivery:";
/// Next delivery sequence number.
pub const NEXT_KEY: &str = "notify:next";
/// Dedupe marker prefix: one (event, channel) pair is enqueued once.
pub const DEDUPE_PREFIX: &str = "notify:dedupe:";
/// Failed attempts before a delivery is dead-lettered.
pub const MAX_ATTEMPTS: u32 = 5;
/// Default request timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Base of the exponential backoff, in seconds.
pub const BACKOFF_BASE_SECS: u64 = 5;
/// Cap of the exponential backoff, in seconds.
pub const BACKOFF_CAP_SECS: u64 = 300;

/// One configured webhook channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WebhookRule {
    pub url: String,
    /// Optional HMAC-SHA256 key for `X-Ferrent-Signature`.
    pub secret: String,
    /// Event kinds to deliver; empty means every kind.
    pub events: Vec<String>,
    pub enabled: bool,
    pub timeout_ms: u64,
}

impl Default for WebhookRule {
    fn default() -> Self {
        Self {
            url: String::new(),
            secret: String::new(),
            events: Vec::new(),
            enabled: true,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl WebhookRule {
    /// Checks the URL scheme and the timeout.
    pub fn validate(&self) -> Result<(), String> {
        let url = self.url.trim();
        if url.is_empty() {
            return Err("webhook url must not be empty".to_owned());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("webhook url must be http(s), got {url:?}"));
        }
        let rest = url
            .split_once("://")
            .map_or("", |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("");
        if rest.is_empty() {
            return Err(format!("webhook url has no host: {url:?}"));
        }
        if self.timeout_ms < 100 || self.timeout_ms > 60_000 {
            return Err("webhook timeout must be between 100 and 60000 ms".to_owned());
        }
        for event in &self.events {
            if EventKind::from_config(event).is_none() {
                return Err(format!("webhook event {event:?} is unknown"));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn wants(&self, kind: EventKind) -> bool {
        self.events.is_empty()
            || self
                .events
                .iter()
                .any(|event| EventKind::from_config(event) == Some(kind))
    }
}

/// One queued delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub seq: u64,
    pub event_seq: u64,
    pub event: EventKind,
    pub torrent_id: u64,
    pub name: String,
    pub infohash: String,
    pub label: String,
    pub url: String,
    pub secret: String,
    pub timeout_ms: u64,
    pub attempt: u32,
    pub next_at: i64,
    pub dead: bool,
    pub last_status: Option<u16>,
    pub last_error: Option<String>,
    pub created_at: i64,
}

impl Delivery {
    /// The JSON body sent to the webhook.
    #[must_use]
    pub fn body(&self) -> String {
        serde_json::json!({
            "event": self.event.as_str(),
            "torrent": {
                "id": self.torrent_id,
                "name": self.name,
                "infohash": self.infohash,
                "label": self.label,
            },
            "delivery": self.seq,
            "at": self.created_at,
        })
        .to_string()
    }

    /// The `X-Ferrent-Signature` value, when a secret is configured.
    #[must_use]
    pub fn signature(&self) -> Option<String> {
        if self.secret.is_empty() {
            None
        } else {
            Some(format!(
                "sha256={}",
                hex(&hmac_sha256(self.secret.as_bytes(), self.body().as_bytes()))
            ))
        }
    }
}

/// What one send attempt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutcome {
    pub status: Option<u16>,
    pub error: Option<String>,
}

impl SendOutcome {
    #[must_use]
    pub fn ok(status: u16) -> Self {
        Self {
            status: Some(status),
            error: None,
        }
    }

    #[must_use]
    pub fn failed(status: Option<u16>, error: impl Into<String>) -> Self {
        Self {
            status,
            error: Some(error.into()),
        }
    }

    /// Whether another attempt can help (transport error, 5xx, 429).
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self.status {
            Some(status) if (200..300).contains(&status) => false,
            Some(429) => true,
            Some(status) if status >= 500 => true,
            Some(_) => false,
            None => true,
        }
    }
}

/// Sends one delivery over HTTP(S).
#[must_use]
pub fn send_http(delivery: &Delivery) -> SendOutcome {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(delivery.timeout_ms)))
        .http_status_as_error(false)
        .build()
        .into();
    let body = delivery.body();
    let mut request = agent
        .post(delivery.url.trim())
        .header("Content-Type", "application/json")
        .header("X-Ferrent-Event", delivery.event.as_str())
        .header("X-Ferrent-Delivery", delivery.seq.to_string());
    if let Some(signature) = delivery.signature() {
        request = request.header("X-Ferrent-Signature", signature);
    }
    match request.send(body.as_bytes()) {
        Ok(response) => SendOutcome::ok(response.status().as_u16()),
        Err(error) => SendOutcome::failed(None, error.to_string()),
    }
}

/// HMAC-SHA256 (RFC 2104) built on the workspace's SHA-256.
#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut padded = [0_u8; 64];
    if key.len() > 64 {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for index in 0..64 {
        inner_pad[index] ^= padded[index];
        outer_pad[index] ^= padded[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn delivery_key(seq: u64) -> String {
    format!("{DELIVERY_PREFIX}{seq:020}")
}

fn dedupe_key(event_seq: u64, url: &str) -> String {
    format!(
        "{DEDUPE_PREFIX}{event_seq}:{}",
        hex(&Sha256::digest(url.as_bytes()))
    )
}

/// Backoff for the next attempt, in seconds.
#[must_use]
pub fn backoff_secs(attempt: u32) -> u64 {
    let factor = 1_u64 << attempt.saturating_sub(1).min(6);
    (BACKOFF_BASE_SECS * factor).min(BACKOFF_CAP_SECS)
}

/// Enqueues one delivery per matching channel; returns the new records.
pub fn enqueue(
    store: &mut dyn StateStore,
    event: &HookEvent,
    label: &str,
    rules: &[WebhookRule],
) -> Result<usize, StoreError> {
    enqueue_at(store, event, label, rules, now_unix())
}

/// Enqueues at a fixed clock, for tests.
pub fn enqueue_at(
    store: &mut dyn StateStore,
    event: &HookEvent,
    label: &str,
    rules: &[WebhookRule],
    now: i64,
) -> Result<usize, StoreError> {
    let mut queued = 0;
    let mut operations = Vec::new();
    let mut next = read_u64(store, NEXT_KEY)?;
    for rule in rules
        .iter()
        .filter(|rule| rule.enabled && rule.wants(event.kind))
    {
        let url = rule.url.trim();
        let dedupe = dedupe_key(event.seq, url);
        if store.get(&dedupe)?.is_some() {
            continue;
        }
        next = next.saturating_add(1);
        let delivery = Delivery {
            seq: next,
            event_seq: event.seq,
            event: event.kind,
            torrent_id: event.torrent_id,
            name: event.name.clone(),
            infohash: event.infohash.clone(),
            label: label.to_owned(),
            url: url.to_owned(),
            secret: rule.secret.clone(),
            timeout_ms: rule.timeout_ms.clamp(100, 60_000),
            attempt: 0,
            next_at: now,
            dead: false,
            last_status: None,
            last_error: None,
            created_at: now,
        };
        let bytes = serde_json::to_vec(&delivery)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        operations.push(StoreOp::Put {
            key: delivery_key(next),
            value: bytes,
        });
        operations.push(StoreOp::Put {
            key: dedupe,
            value: next.to_le_bytes().to_vec(),
        });
        queued += 1;
    }
    if !operations.is_empty() {
        operations.push(StoreOp::Put {
            key: NEXT_KEY.to_owned(),
            value: next.to_le_bytes().to_vec(),
        });
        store.batch(operations)?;
    }
    Ok(queued)
}

fn read_u64(store: &dyn StateStore, key: &str) -> Result<u64, StoreError> {
    Ok(store
        .get(key)?
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0))
}

/// Every queued delivery, oldest first.
pub fn deliveries(store: &dyn StateStore) -> Result<Vec<Delivery>, StoreError> {
    let mut items: Vec<Delivery> = store
        .list(DELIVERY_PREFIX)?
        .into_iter()
        .filter_map(|(_, bytes)| serde_json::from_slice(&bytes).ok())
        .collect();
    items.sort_by_key(|delivery| delivery.seq);
    Ok(items)
}

/// Counts for the status RPC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxStatus {
    pub pending: u32,
    pub dead: u32,
    pub sent: u32,
}

/// Summary of the outbox.
pub fn status(store: &dyn StateStore) -> Result<OutboxStatus, StoreError> {
    let mut status = OutboxStatus::default();
    for delivery in deliveries(store)? {
        if delivery.dead {
            status.dead = status.dead.saturating_add(1);
        } else {
            status.pending = status.pending.saturating_add(1);
        }
    }
    Ok(status)
}

/// Processes every due delivery; returns the counts.
pub fn process(
    store: &mut dyn StateStore,
    now: i64,
    send: &mut dyn FnMut(&Delivery) -> SendOutcome,
) -> Result<OutboxStatus, StoreError> {
    let mut report = OutboxStatus::default();
    for mut delivery in deliveries(store)? {
        if delivery.dead || delivery.next_at > now || delivery.attempt >= MAX_ATTEMPTS {
            continue;
        }
        let outcome = send(&delivery);
        delivery.attempt = delivery.attempt.saturating_add(1);
        delivery.last_status = outcome.status;
        delivery.last_error = outcome.error.clone();
        if outcome
            .status
            .is_some_and(|status| (200..300).contains(&status))
        {
            store.delete(&delivery_key(delivery.seq))?;
            report.sent = report.sent.saturating_add(1);
            continue;
        }
        if !outcome.retryable() || delivery.attempt >= MAX_ATTEMPTS {
            delivery.dead = true;
            report.dead = report.dead.saturating_add(1);
        } else {
            let backoff = i64::try_from(backoff_secs(delivery.attempt)).unwrap_or(i64::MAX);
            delivery.next_at = now.saturating_add(backoff);
            report.pending = report.pending.saturating_add(1);
        }
        let bytes = serde_json::to_vec(&delivery)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        store.put(&delivery_key(delivery.seq), &bytes)?;
    }
    Ok(report)
}

/// Removes the dead letters (manual retry/reset from the GUI).
pub fn clear_dead(store: &mut dyn StateStore) -> Result<u32, StoreError> {
    let dead: BTreeSet<u64> = deliveries(store)?
        .into_iter()
        .filter(|delivery| delivery.dead)
        .map(|delivery| delivery.seq)
        .collect();
    let count = u32::try_from(dead.len()).unwrap_or(u32::MAX);
    let operations = dead
        .into_iter()
        .map(|seq| StoreOp::Delete {
            key: delivery_key(seq),
        })
        .collect::<Vec<_>>();
    if !operations.is_empty() {
        store.batch(operations)?;
    }
    Ok(count)
}

impl crate::rpc::AppState {
    /// Appends an event to the hook journal and enqueues its notifications.
    pub async fn publish_event(&self, event: crate::events::NewEvent) -> Result<HookEvent, String> {
        let stored = crate::hooks::record_event(Arc::clone(&self.store), event).await?;
        let label = self
            .labels
            .lock()
            .expect("label state poisoned")
            .label(stored.torrent_id)
            .unwrap_or_default()
            .to_owned();
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        if config.notify.enabled && !config.notify.webhook.is_empty() {
            let queued = stored.clone();
            let rules = config.notify.webhook.clone();
            let store = Arc::clone(&self.store);
            tokio::task::spawn_blocking(move || {
                let mut store = store.lock().expect("state store mutex poisoned");
                enqueue(&mut **store, &queued, &label, &rules)
                    .map_err(|error| error.to_string())?;
                store.flush().map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
        }
        Ok(stored)
    }

    /// Queues one synthetic delivery per enabled channel (the GUI/CLI test).
    pub async fn send_test_notification(&self) -> Result<u32, String> {
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        let rules = config.notify.webhook;
        if rules.is_empty() {
            return Ok(0);
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        let event = HookEvent {
            seq: u64::try_from(now).unwrap_or(u64::MAX),
            kind: EventKind::Finished,
            torrent_id: 0,
            name: "Ferrent test notification".to_owned(),
            save_path: String::new(),
            infohash: String::new(),
            error: None,
            at_unix: now / 1000,
        };
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            let queued = enqueue_at(&mut **store, &event, "", &rules, now / 1000)
                .map_err(|error| error.to_string())?;
            store.flush().map_err(|error| error.to_string())?;
            Ok(u32::try_from(queued).unwrap_or(u32::MAX))
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Outbox counters for the status RPC.
    pub async fn notification_status(&self) -> Result<OutboxStatus, String> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let store = store.lock().expect("state store mutex poisoned");
            status(&**store).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// Runs one router pass now (used by the loop and the tests).
    pub async fn notify_tick(&self, now: i64) -> Result<OutboxStatus, String> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            let report =
                process(&mut **store, now, &mut send_http).map_err(|error| error.to_string())?;
            store.flush().map_err(|error| error.to_string())?;
            Ok(report)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

/// Starts the notification router loop.
pub fn spawn_notifier(state: Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = state.notify_tick(now_unix()).await {
                eprintln!("notification router: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NewEvent;
    use crate::store::MemoryStore;

    fn event(seq: u64) -> HookEvent {
        HookEvent {
            seq,
            kind: EventKind::Finished,
            torrent_id: 7,
            name: "movie.mkv".to_owned(),
            save_path: "/data".to_owned(),
            infohash: "ab".repeat(20),
            error: None,
            at_unix: 1,
        }
    }

    fn rule(url: &str) -> WebhookRule {
        WebhookRule {
            url: url.to_owned(),
            ..WebhookRule::default()
        }
    }

    #[test]
    fn hmac_matches_rfc_4231_case_two() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn enqueue_is_deduped_per_event_and_channel() {
        let mut store = MemoryStore::new();
        let rules = vec![
            rule("https://one.example/hook"),
            rule("https://two.example/hook"),
        ];
        assert_eq!(
            enqueue_at(&mut store, &event(1), "movies", &rules, 100).expect("queue"),
            2
        );
        assert_eq!(
            enqueue_at(&mut store, &event(1), "movies", &rules, 100).expect("queue again"),
            0,
            "the same (event, channel) is enqueued once"
        );
        let queued = deliveries(&store).expect("deliveries");
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].label, "movies");
        assert_eq!(queued[0].attempt, 0);
    }

    #[test]
    fn rules_filter_by_event_and_enabled() {
        let mut store = MemoryStore::new();
        let rules = vec![
            WebhookRule {
                events: vec!["error".to_owned()],
                ..rule("https://one.example/hook")
            },
            WebhookRule {
                enabled: false,
                ..rule("https://two.example/hook")
            },
            rule("https://three.example/hook"),
        ];
        assert_eq!(
            enqueue_at(&mut store, &event(1), "", &rules, 1).expect("queue"),
            1
        );
        let queued = deliveries(&store).expect("deliveries");
        assert_eq!(queued[0].url, "https://three.example/hook");
    }

    #[test]
    fn failures_back_off_and_dead_letter_after_five_attempts() {
        let mut store = MemoryStore::new();
        let rules = vec![rule("https://one.example/hook")];
        enqueue_at(&mut store, &event(1), "", &rules, 0).expect("queue");

        let mut now = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let report = process(&mut store, now, &mut |_| {
                SendOutcome::failed(Some(500), "boom")
            })
            .expect("process");
            assert_eq!(report.pending, u32::from(attempt < MAX_ATTEMPTS));
            assert_eq!(report.dead, u32::from(attempt == MAX_ATTEMPTS));
            let delivery = &deliveries(&store).expect("deliveries")[0];
            if attempt < MAX_ATTEMPTS {
                assert!(!delivery.dead);
                assert_eq!(delivery.attempt, attempt);
                assert_eq!(
                    delivery.next_at,
                    now + i64::try_from(backoff_secs(attempt)).unwrap()
                );
                now = delivery.next_at;
            } else {
                assert!(delivery.dead);
                assert_eq!(delivery.last_status, Some(500));
            }
        }
        // A dead letter is not retried.
        let report =
            process(&mut store, now + 10_000, &mut |_| SendOutcome::ok(200)).expect("process");
        assert_eq!(report.sent, 0);
        assert_eq!(clear_dead(&mut store).expect("clear"), 1);
        assert!(deliveries(&store).expect("deliveries").is_empty());
    }

    #[test]
    fn a_two_hundred_acknowledges_and_a_four_hundred_dead_letters() {
        let mut store = MemoryStore::new();
        let rules = vec![rule("https://one.example/hook")];
        enqueue_at(&mut store, &event(1), "", &rules, 0).expect("queue");
        process(&mut store, 0, &mut |_| SendOutcome::ok(204)).expect("process");
        assert!(deliveries(&store).expect("deliveries").is_empty());

        enqueue_at(&mut store, &event(2), "", &rules, 0).expect("queue");
        process(&mut store, 0, &mut |_| {
            SendOutcome::failed(Some(400), "bad")
        })
        .expect("process");
        assert!(deliveries(&store).expect("deliveries")[0].dead);
    }

    #[test]
    fn the_signature_covers_the_body() {
        let mut delivery = Delivery {
            seq: 3,
            event_seq: 1,
            event: EventKind::Finished,
            torrent_id: 1,
            name: "a".to_owned(),
            infohash: String::new(),
            label: String::new(),
            url: "https://x/y".to_owned(),
            secret: "s3cret".to_owned(),
            timeout_ms: 1000,
            attempt: 0,
            next_at: 0,
            dead: false,
            last_status: None,
            last_error: None,
            created_at: 9,
        };
        let signature = delivery.signature().expect("signature");
        let expected = format!(
            "sha256={}",
            hex(&hmac_sha256(b"s3cret", delivery.body().as_bytes()))
        );
        assert_eq!(signature, expected);
        assert!(signature.starts_with("sha256="));
        delivery.secret.clear();
        assert!(delivery.signature().is_none());
    }

    #[test]
    fn urls_are_limited_to_http_and_https() {
        let mut config = rule("ftp://example/hook");
        assert!(config.validate().is_err());
        config.url = "https://example/hook".to_owned();
        assert!(config.validate().is_ok());
        config.url = "http://example/hook".to_owned();
        assert!(config.validate().is_ok());
        config.url = "https://".to_owned();
        assert!(config.validate().is_err());
        config.url = "https://example/hook".to_owned();
        config.timeout_ms = 1;
        assert!(config.validate().is_err());
        config.timeout_ms = DEFAULT_TIMEOUT_MS;
        config.events = vec!["nope".to_owned()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff_secs(1), 5);
        assert_eq!(backoff_secs(2), 10);
        assert_eq!(backoff_secs(3), 20);
        assert_eq!(backoff_secs(4), 40);
        assert_eq!(backoff_secs(5), 80);
        assert_eq!(backoff_secs(20), BACKOFF_CAP_SECS);
    }

    #[test]
    fn a_new_event_can_reuse_a_delivery_slot() {
        // The dedupe marker is per event; a later event enqueues again.
        let mut store = MemoryStore::new();
        let rules = vec![rule("https://one.example/hook")];
        enqueue_at(&mut store, &event(1), "", &rules, 0).expect("queue");
        enqueue_at(&mut store, &event(2), "", &rules, 0).expect("queue");
        assert_eq!(deliveries(&store).expect("deliveries").len(), 2);
        assert!(NewEvent::new(EventKind::Added, 1, "x").name == "x");
    }
}
