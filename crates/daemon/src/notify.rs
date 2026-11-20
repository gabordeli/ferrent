//! Notification router: events → rules → durable outbox → webhook/Telegram.
//!
//! One record per (event, channel) is written before delivery, so a restart
//! resumes instead of duplicating. Webhooks POST JSON with an optional
//! HMAC signature; Telegram uses the Bot API with `retry_after` support.
//! Failures back off exponentially and dead-letter after [`MAX_ATTEMPTS`].
//! Telegram deliveries can be digested and held during quiet hours; webhooks
//! are always immediate.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::events::{EventKind, HookEvent};
use crate::store::{StateStore, StoreError, StoreOp};

pub const DELIVERY_PREFIX: &str = "notify:delivery:";
pub const NEXT_KEY: &str = "notify:next";
pub const DEDUPE_PREFIX: &str = "notify:dedupe:";
pub const MAX_ATTEMPTS: u32 = 5;
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
pub const BACKOFF_BASE_SECS: u64 = 5;
pub const BACKOFF_CAP_SECS: u64 = 300;
/// Telegram's message limit.
pub const TELEGRAM_MAX_CHARS: usize = 4_096;
/// Default API root (overridable per channel for tests).
pub const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// One configured webhook channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WebhookRule {
    pub url: String,
    pub secret: String,
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
    pub fn validate(&self) -> Result<(), String> {
        let url = self.url.trim();
        if url.is_empty() {
            return Err("webhook url must not be empty".to_owned());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!("webhook url must be http(s), got {url:?}"));
        }
        if url
            .split_once("://")
            .map_or("", |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("")
            .is_empty()
        {
            return Err(format!("webhook url has no host: {url:?}"));
        }
        if !(100..=60_000).contains(&self.timeout_ms) {
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

/// One configured Telegram channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TelegramRule {
    pub bot_token: String,
    pub chat_id: String,
    /// Forum topic id; empty for the main chat.
    pub message_thread_id: String,
    pub events: Vec<String>,
    /// Only deliver events with this label (empty = every label).
    pub label: String,
    pub enabled: bool,
    /// API root; overridable for tests.
    pub api_base: String,
    pub timeout_ms: u64,
}

impl Default for TelegramRule {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            chat_id: String::new(),
            message_thread_id: String::new(),
            events: Vec::new(),
            label: String::new(),
            enabled: true,
            api_base: TELEGRAM_API_BASE.to_owned(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl TelegramRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.bot_token.trim().is_empty() {
            return Err("telegram bot_token must not be empty".to_owned());
        }
        if self.chat_id.trim().is_empty() {
            return Err("telegram chat_id must not be empty".to_owned());
        }
        let base = self.api_base.trim();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err("telegram api_base must be http(s)".to_owned());
        }
        if !(100..=60_000).contains(&self.timeout_ms) {
            return Err("telegram timeout must be between 100 and 60000 ms".to_owned());
        }
        for event in &self.events {
            if EventKind::from_config(event).is_none() {
                return Err(format!("telegram event {event:?} is unknown"));
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

    #[must_use]
    pub fn allows_label(&self, label: &str) -> bool {
        self.label.trim().is_empty() || self.label.trim() == label
    }
}

/// The configured rules for one enqueue pass.
#[derive(Debug, Clone, Copy)]
pub struct NotifyRules<'a> {
    pub webhooks: &'a [WebhookRule],
    pub telegram: &'a [TelegramRule],
    /// Global label filter; empty accepts every label.
    pub label_filter: &'a str,
}

impl NotifyRules<'_> {
    #[must_use]
    pub fn allows_label(&self, label: &str) -> bool {
        self.label_filter.trim().is_empty() || self.label_filter.trim() == label
    }
}

/// The Telegram endpoint of one delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramTarget {
    pub api_base: String,
    pub bot_token: String,
    pub chat_id: String,
    pub message_thread_id: String,
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
    /// Set for Telegram deliveries.
    #[serde(default)]
    pub telegram: Option<TelegramTarget>,
    pub timeout_ms: u64,
    pub attempt: u32,
    pub next_at: i64,
    pub dead: bool,
    pub last_status: Option<u16>,
    pub last_error: Option<String>,
    pub created_at: i64,
}

impl Delivery {
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

    /// The Telegram message for one delivery.
    #[must_use]
    pub fn telegram_text(&self) -> String {
        let mut text = format!(
            "<b>{}</b>\n{}",
            escape_html(self.event.as_str()),
            escape_html(&self.name)
        );
        if !self.label.is_empty() {
            text.push_str(&format!("\nlabel: {}", escape_html(&self.label)));
        }
        truncate_chars(&text, TELEGRAM_MAX_CHARS)
    }

    fn channel_key(&self) -> String {
        match &self.telegram {
            Some(target) => format!("telegram:{}:{}", target.chat_id, target.message_thread_id),
            None => format!("webhook:{}", self.url),
        }
    }
}

/// What one send attempt produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutcome {
    pub status: Option<u16>,
    pub error: Option<String>,
    /// Telegram 429 hint.
    pub retry_after: Option<u64>,
}

impl SendOutcome {
    #[must_use]
    pub fn ok(status: u16) -> Self {
        Self {
            status: Some(status),
            error: None,
            retry_after: None,
        }
    }

    #[must_use]
    pub fn failed(status: Option<u16>, error: impl Into<String>) -> Self {
        Self {
            status,
            error: Some(error.into()),
            retry_after: None,
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

fn agent(timeout_ms: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout_ms.clamp(100, 60_000))))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Sends one delivery (webhook JSON or Telegram message).
#[must_use]
pub fn send(delivery: &Delivery, text: Option<&str>) -> SendOutcome {
    match &delivery.telegram {
        Some(target) => send_telegram(delivery, target, text),
        None => send_webhook(delivery),
    }
}

/// Sends the webhook JSON body.
#[must_use]
pub fn send_webhook(delivery: &Delivery) -> SendOutcome {
    let agent = agent(delivery.timeout_ms);
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

/// Sends one Telegram message; on 429 it reports the server's `retry_after`.
#[must_use]
pub fn send_telegram(
    delivery: &Delivery,
    target: &TelegramTarget,
    text: Option<&str>,
) -> SendOutcome {
    let agent = agent(delivery.timeout_ms);
    let url = format!(
        "{}/bot{}/sendMessage",
        target.api_base.trim_end_matches('/'),
        target.bot_token
    );
    let mut payload = serde_json::json!({
        "chat_id": target.chat_id,
        "text": text.map_or_else(|| delivery.telegram_text(), str::to_owned),
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    });
    if !target.message_thread_id.trim().is_empty() {
        payload["message_thread_id"] = serde_json::Value::String(target.message_thread_id.clone());
    }
    let body = payload.to_string();
    match agent
        .post(&url)
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
    {
        Ok(response) => {
            let status = response.status().as_u16();
            if status == 429 {
                let body = response.into_body().read_to_string().unwrap_or_default();
                let retry_after = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("parameters")
                            .and_then(|parameters| parameters.get("retry_after"))
                            .and_then(serde_json::Value::as_u64)
                    });
                return SendOutcome {
                    status: Some(429),
                    error: Some("telegram rate limit".to_owned()),
                    retry_after,
                };
            }
            SendOutcome::ok(status)
        }
        Err(error) => SendOutcome::failed(None, error.to_string()),
    }
}

/// HMAC-SHA256 (RFC 2104).
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

fn dedupe_key(event_seq: u64, channel: &str) -> String {
    format!(
        "{DEDUPE_PREFIX}{event_seq}:{}",
        hex(&Sha256::digest(channel.as_bytes()))
    )
}

/// Backoff for the next attempt, in seconds.
#[must_use]
pub fn backoff_secs(attempt: u32) -> u64 {
    let factor = 1_u64 << attempt.saturating_sub(1).min(6);
    (BACKOFF_BASE_SECS * factor).min(BACKOFF_CAP_SECS)
}

/// Enqueues one delivery per matching channel.
pub fn enqueue(
    store: &mut dyn StateStore,
    event: &HookEvent,
    label: &str,
    rules: NotifyRules<'_>,
) -> Result<usize, StoreError> {
    enqueue_at(store, event, label, rules, now_unix())
}

/// Enqueues at a fixed clock, for tests.
pub fn enqueue_at(
    store: &mut dyn StateStore,
    event: &HookEvent,
    label: &str,
    rules: NotifyRules<'_>,
    now: i64,
) -> Result<usize, StoreError> {
    if !rules.allows_label(label) {
        return Ok(0);
    }
    let mut queued = 0;
    let mut operations = Vec::new();
    let mut next = read_u64(store, NEXT_KEY)?;
    for rule in rules
        .webhooks
        .iter()
        .filter(|rule| rule.enabled && rule.wants(event.kind))
    {
        let url = rule.url.trim();
        let dedupe = dedupe_key(event.seq, &format!("webhook:{url}"));
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
            telegram: None,
            timeout_ms: rule.timeout_ms.clamp(100, 60_000),
            attempt: 0,
            next_at: now,
            dead: false,
            last_status: None,
            last_error: None,
            created_at: now,
        };
        push_delivery(&mut operations, &delivery, dedupe)?;
        queued += 1;
    }
    for rule in rules
        .telegram
        .iter()
        .filter(|rule| rule.enabled && rule.wants(event.kind) && rule.allows_label(label))
    {
        let target = TelegramTarget {
            api_base: rule.api_base.trim().to_owned(),
            bot_token: rule.bot_token.trim().to_owned(),
            chat_id: rule.chat_id.trim().to_owned(),
            message_thread_id: rule.message_thread_id.trim().to_owned(),
        };
        let dedupe = dedupe_key(
            event.seq,
            &format!("telegram:{}:{}", target.chat_id, target.message_thread_id),
        );
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
            url: String::new(),
            secret: String::new(),
            telegram: Some(target),
            timeout_ms: rule.timeout_ms.clamp(100, 60_000),
            attempt: 0,
            next_at: now,
            dead: false,
            last_status: None,
            last_error: None,
            created_at: now,
        };
        push_delivery(&mut operations, &delivery, dedupe)?;
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

fn push_delivery(
    operations: &mut Vec<StoreOp>,
    delivery: &Delivery,
    dedupe: String,
) -> Result<(), StoreError> {
    let bytes =
        serde_json::to_vec(delivery).map_err(|error| StoreError::Backend(error.to_string()))?;
    operations.push(StoreOp::Put {
        key: delivery_key(delivery.seq),
        value: bytes,
    });
    operations.push(StoreOp::Put {
        key: dedupe,
        value: delivery.seq.to_le_bytes().to_vec(),
    });
    Ok(())
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboxStatus {
    pub pending: u32,
    pub dead: u32,
    pub sent: u32,
}

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

/// Digest and quiet-hours policy for Telegram deliveries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigestPolicy {
    /// Send a digest after this many events; `0` sends immediately.
    pub events: usize,
    /// Send a digest after this many seconds even below `events`.
    pub window_secs: u64,
    /// Quiet hours as local minutes `[start, end)`.
    pub quiet: Option<(u32, u32)>,
    /// The local minute of `now` in the notification timezone.
    pub local_minute: u32,
}

impl DigestPolicy {
    #[must_use]
    pub fn quiet_now(&self) -> bool {
        match self.quiet {
            Some((start, end)) if start == end => false,
            Some((start, end)) if start < end => (start..end).contains(&self.local_minute),
            Some((start, end)) => self.local_minute >= start || self.local_minute < end,
            None => false,
        }
    }

    /// Minutes until quiet hours end (0 when not quiet).
    #[must_use]
    pub fn minutes_until_quiet_end(&self) -> u64 {
        match self.quiet {
            Some((_, end)) if self.quiet_now() => {
                u64::from((end + 1440 - self.local_minute % 1440) % 1440)
            }
            _ => 0,
        }
    }
}

/// Processes every due delivery.
pub fn process(
    store: &mut dyn StateStore,
    now: i64,
    policy: DigestPolicy,
    send: &mut dyn FnMut(&Delivery, Option<&str>) -> SendOutcome,
) -> Result<OutboxStatus, StoreError> {
    let mut report = OutboxStatus::default();
    let mut telegram: BTreeMap<String, Vec<Delivery>> = BTreeMap::new();
    let pending = deliveries(store)?;
    for mut delivery in pending {
        if delivery.dead || delivery.next_at > now || delivery.attempt >= MAX_ATTEMPTS {
            continue;
        }
        if delivery.telegram.is_some() && policy.events > 0 {
            telegram
                .entry(delivery.channel_key())
                .or_default()
                .push(delivery);
            continue;
        }
        apply_outcome(store, &mut delivery, now, policy, send, &mut report)?;
    }

    for (_, group) in telegram {
        if policy.quiet_now() {
            let until = i64::try_from(policy.minutes_until_quiet_end() * 60).unwrap_or(0);
            for mut delivery in group {
                delivery.next_at = now.saturating_add(until.max(1));
                let bytes = serde_json::to_vec(&delivery)
                    .map_err(|error| StoreError::Backend(error.to_string()))?;
                store.put(&delivery_key(delivery.seq), &bytes)?;
            }
            continue;
        }
        let oldest = group
            .iter()
            .map(|item| item.created_at)
            .min()
            .unwrap_or(now);
        let age = u64::try_from(now.saturating_sub(oldest)).unwrap_or(0);
        let ready =
            group.len() >= policy.events || (policy.window_secs > 0 && age >= policy.window_secs);
        if !ready {
            continue;
        }
        for batch in group.chunks(policy.events.max(1)) {
            let text = digest_text(batch);
            let outcome = send(&batch[0], Some(&text));
            if outcome
                .status
                .is_some_and(|status| (200..300).contains(&status))
            {
                for delivery in batch {
                    store.delete(&delivery_key(delivery.seq))?;
                    report.sent = report.sent.saturating_add(1);
                }
                continue;
            }
            for delivery in batch {
                let mut delivery = delivery.clone();
                delivery.attempt = delivery.attempt.saturating_add(1);
                delivery.last_status = outcome.status;
                delivery.last_error = outcome.error.clone();
                if !outcome.retryable() || delivery.attempt >= MAX_ATTEMPTS {
                    delivery.dead = true;
                    report.dead = report.dead.saturating_add(1);
                } else {
                    let seconds = outcome
                        .retry_after
                        .unwrap_or_else(|| backoff_secs(delivery.attempt));
                    let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
                    delivery.next_at = now.saturating_add(seconds.max(1));
                    report.pending = report.pending.saturating_add(1);
                }
                let bytes = serde_json::to_vec(&delivery)
                    .map_err(|error| StoreError::Backend(error.to_string()))?;
                store.put(&delivery_key(delivery.seq), &bytes)?;
            }
        }
    }
    Ok(report)
}

fn apply_outcome(
    store: &mut dyn StateStore,
    delivery: &mut Delivery,
    now: i64,
    policy: DigestPolicy,
    send: &mut dyn FnMut(&Delivery, Option<&str>) -> SendOutcome,
    report: &mut OutboxStatus,
) -> Result<(), StoreError> {
    if delivery.telegram.is_some() && policy.quiet_now() {
        let until = i64::try_from(policy.minutes_until_quiet_end() * 60).unwrap_or(0);
        delivery.next_at = now.saturating_add(until.max(1));
        let bytes =
            serde_json::to_vec(delivery).map_err(|error| StoreError::Backend(error.to_string()))?;
        store.put(&delivery_key(delivery.seq), &bytes)?;
        return Ok(());
    }
    let outcome = send(delivery, None);
    delivery.attempt = delivery.attempt.saturating_add(1);
    delivery.last_status = outcome.status;
    delivery.last_error = outcome.error.clone();
    if outcome
        .status
        .is_some_and(|status| (200..300).contains(&status))
    {
        store.delete(&delivery_key(delivery.seq))?;
        report.sent = report.sent.saturating_add(1);
        return Ok(());
    }
    if !outcome.retryable() || delivery.attempt >= MAX_ATTEMPTS {
        delivery.dead = true;
        report.dead = report.dead.saturating_add(1);
    } else {
        let seconds = outcome
            .retry_after
            .unwrap_or_else(|| backoff_secs(delivery.attempt));
        let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
        delivery.next_at = now.saturating_add(seconds.max(1));
        report.pending = report.pending.saturating_add(1);
    }
    let bytes =
        serde_json::to_vec(delivery).map_err(|error| StoreError::Backend(error.to_string()))?;
    store.put(&delivery_key(delivery.seq), &bytes)?;
    Ok(())
}

/// One digest message for a group of Telegram deliveries.
#[must_use]
pub fn digest_text(group: &[Delivery]) -> String {
    let mut text = format!("<b>Ferrent</b> · {} event(s)", group.len());
    for delivery in group {
        text.push_str(&format!(
            "\n<b>{}</b> {}",
            escape_html(delivery.event.as_str()),
            escape_html(&delivery.name)
        ));
    }
    truncate_chars(&text, TELEGRAM_MAX_CHARS)
}

/// Escapes the three characters Telegram's HTML mode interprets.
#[must_use]
pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncates to `max` characters (not bytes) with an ellipsis.
#[must_use]
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
}

/// Removes the dead letters.
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
        if config.notify.enabled
            && (!config.notify.webhook.is_empty() || !config.notify.telegram.is_empty())
        {
            let queued = stored.clone();
            let webhooks = config.notify.webhook.clone();
            let telegram = config.notify.telegram.clone();
            let label_filter = config.notify.label_filter.clone();
            let store = Arc::clone(&self.store);
            tokio::task::spawn_blocking(move || {
                let rules = NotifyRules {
                    webhooks: &webhooks,
                    telegram: &telegram,
                    label_filter: &label_filter,
                };
                let mut store = store.lock().expect("state store mutex poisoned");
                enqueue(&mut **store, &queued, &label, rules).map_err(|error| error.to_string())?;
                store.flush().map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
        }
        Ok(stored)
    }

    /// Queues one synthetic delivery per enabled channel.
    pub async fn send_test_notification(&self) -> Result<u32, String> {
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        let webhooks = config.notify.webhook.clone();
        let telegram = config.notify.telegram.clone();
        if webhooks.is_empty() && telegram.is_empty() {
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
            let rules = NotifyRules {
                webhooks: &webhooks,
                telegram: &telegram,
                label_filter: "",
            };
            let queued = enqueue_at(&mut **store, &event, "", rules, now / 1000)
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
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        let policy = digest_policy(&config.notify)?;
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            let report =
                process(&mut **store, now, policy, &mut send).map_err(|error| error.to_string())?;
            store.flush().map_err(|error| error.to_string())?;
            Ok(report)
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

/// Builds the digest/quiet policy for the current time.
fn digest_policy(config: &crate::config::NotifyConfig) -> Result<DigestPolicy, String> {
    let timezone: chrono_tz::Tz = config
        .timezone
        .trim()
        .parse()
        .map_err(|_| format!("unknown notification timezone {:?}", config.timezone))?;
    let local_minute = {
        use chrono::Timelike;
        let now = chrono::Utc::now().with_timezone(&timezone);
        now.hour() * 60 + now.minute()
    };
    Ok(DigestPolicy {
        events: usize::try_from(config.digest_events).unwrap_or(0),
        window_secs: config.digest_window_secs,
        quiet: parse_quiet(&config.quiet_start, &config.quiet_end)?,
        local_minute,
    })
}

/// Parses `HH:MM` quiet-hour bounds.
pub fn parse_quiet(start: &str, end: &str) -> Result<Option<(u32, u32)>, String> {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() && end.is_empty() {
        return Ok(None);
    }
    let parse = |text: &str, name: &str| -> Result<u32, String> {
        let (hours, minutes) = text
            .split_once(':')
            .ok_or_else(|| format!("{name} must be HH:MM"))?;
        let hours: u32 = hours
            .parse()
            .map_err(|_| format!("{name} hour is invalid"))?;
        let minutes: u32 = minutes
            .parse()
            .map_err(|_| format!("{name} minute is invalid"))?;
        if hours > 23 || minutes > 59 {
            return Err(format!("{name} is out of range"));
        }
        Ok(hours * 60 + minutes)
    };
    Ok(Some((
        parse(start, "quiet_start")?,
        parse(end, "quiet_end")?,
    )))
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

    fn webhook(url: &str) -> WebhookRule {
        WebhookRule {
            url: url.to_owned(),
            ..WebhookRule::default()
        }
    }

    fn telegram(chat: &str) -> TelegramRule {
        TelegramRule {
            bot_token: "token".to_owned(),
            chat_id: chat.to_owned(),
            ..TelegramRule::default()
        }
    }

    fn rules<'a>(
        webhooks: &'a [WebhookRule],
        telegram_rules: &'a [TelegramRule],
    ) -> NotifyRules<'a> {
        NotifyRules {
            webhooks,
            telegram: telegram_rules,
            label_filter: "",
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
        let webhooks = vec![
            webhook("https://one.example/hook"),
            webhook("https://two.example/hook"),
        ];
        let telegram_rules = vec![telegram("42")];
        assert_eq!(
            enqueue_at(
                &mut store,
                &event(1),
                "movies",
                rules(&webhooks, &telegram_rules),
                100
            )
            .expect("queue"),
            3
        );
        assert_eq!(
            enqueue_at(
                &mut store,
                &event(1),
                "movies",
                rules(&webhooks, &telegram_rules),
                100
            )
            .expect("queue again"),
            0
        );
        let queued = deliveries(&store).expect("deliveries");
        assert_eq!(queued.len(), 3);
        assert!(queued.iter().any(|delivery| delivery.telegram.is_some()));
    }

    #[test]
    fn labels_and_events_filter_the_channels() {
        let mut store = MemoryStore::new();
        let webhooks = vec![WebhookRule {
            events: vec!["error".to_owned()],
            ..webhook("https://one.example/hook")
        }];
        let telegram_rules = vec![TelegramRule {
            label: "movies".to_owned(),
            ..telegram("42")
        }];
        assert_eq!(
            enqueue_at(
                &mut store,
                &event(1),
                "series",
                rules(&webhooks, &telegram_rules),
                1
            )
            .expect("queue"),
            0
        );
        assert_eq!(
            enqueue_at(
                &mut store,
                &event(2),
                "movies",
                rules(&webhooks, &telegram_rules),
                1
            )
            .expect("queue"),
            1
        );
        let global = NotifyRules {
            webhooks: &[],
            telegram: &telegram_rules,
            label_filter: "series",
        };
        assert_eq!(
            enqueue_at(&mut store, &event(3), "movies", global, 1).expect("queue"),
            0
        );
    }

    #[test]
    fn failures_back_off_and_dead_letter_after_five_attempts() {
        let mut store = MemoryStore::new();
        let webhooks = vec![webhook("https://one.example/hook")];
        enqueue_at(&mut store, &event(1), "", rules(&webhooks, &[]), 0).expect("queue");

        let mut now = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let report = process(&mut store, now, DigestPolicy::default(), &mut |_, _| {
                SendOutcome::failed(Some(500), "boom")
            })
            .expect("process");
            assert_eq!(report.pending, u32::from(attempt < MAX_ATTEMPTS));
            assert_eq!(report.dead, u32::from(attempt == MAX_ATTEMPTS));
            let delivery = &deliveries(&store).expect("deliveries")[0];
            if attempt < MAX_ATTEMPTS {
                assert!(!delivery.dead);
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
        let report = process(
            &mut store,
            now + 10_000,
            DigestPolicy::default(),
            &mut |_, _| SendOutcome::ok(200),
        )
        .expect("process");
        assert_eq!(report.sent, 0);
        assert_eq!(clear_dead(&mut store).expect("clear"), 1);
        assert!(deliveries(&store).expect("deliveries").is_empty());
    }

    #[test]
    fn a_two_hundred_acknowledges_and_a_four_hundred_dead_letters() {
        let mut store = MemoryStore::new();
        let webhooks = vec![webhook("https://one.example/hook")];
        enqueue_at(&mut store, &event(1), "", rules(&webhooks, &[]), 0).expect("queue");
        process(&mut store, 0, DigestPolicy::default(), &mut |_, _| {
            SendOutcome::ok(204)
        })
        .expect("process");
        assert!(deliveries(&store).expect("deliveries").is_empty());

        enqueue_at(&mut store, &event(2), "", rules(&webhooks, &[]), 0).expect("queue");
        process(&mut store, 0, DigestPolicy::default(), &mut |_, _| {
            SendOutcome::failed(Some(400), "bad")
        })
        .expect("process");
        assert!(deliveries(&store).expect("deliveries")[0].dead);
    }

    #[test]
    fn telegram_429_waits_the_server_hint() {
        let mut store = MemoryStore::new();
        enqueue_at(&mut store, &event(1), "", rules(&[], &[telegram("42")]), 0).expect("queue");
        let outcome = SendOutcome {
            status: Some(429),
            error: Some("rate limit".to_owned()),
            retry_after: Some(7),
        };
        let report = process(&mut store, 0, DigestPolicy::default(), &mut |_, _| {
            outcome.clone()
        })
        .expect("process");
        assert_eq!(report.pending, 1);
        let delivery = &deliveries(&store).expect("deliveries")[0];
        assert_eq!(
            delivery.next_at, 7,
            "retry_after wins over the exponential backoff"
        );
    }

    #[test]
    fn quiet_hours_hold_telegram_but_not_webhooks() {
        let mut store = MemoryStore::new();
        let webhooks = vec![webhook("https://one.example/hook")];
        enqueue_at(
            &mut store,
            &event(1),
            "",
            rules(&webhooks, &[telegram("42")]),
            0,
        )
        .expect("queue");
        let policy = DigestPolicy {
            quiet: Some((23 * 60, 7 * 60)),
            local_minute: 23 * 60 + 30,
            ..DigestPolicy::default()
        };
        let report =
            process(&mut store, 0, policy, &mut |_, _| SendOutcome::ok(200)).expect("process");
        assert_eq!(report.sent, 1, "the webhook is immediate");
        let remaining = deliveries(&store).expect("deliveries");
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].telegram.is_some());
        assert_eq!(remaining[0].next_at, 450 * 60, "held until 07:00 local");
    }

    #[test]
    fn digest_waits_for_ten_events_or_the_window() {
        let mut store = MemoryStore::new();
        let telegram_rules = vec![telegram("42")];
        for seq in 1..=100 {
            enqueue_at(&mut store, &event(seq), "", rules(&[], &telegram_rules), 0).expect("queue");
        }
        let policy = DigestPolicy {
            events: 10,
            window_secs: 30,
            ..DigestPolicy::default()
        };
        let mut texts = Vec::new();
        let report = process(&mut store, 0, policy, &mut |_, text| {
            texts.push(text.unwrap_or_default().to_owned());
            SendOutcome::ok(200)
        })
        .expect("process");
        assert_eq!(report.sent, 100);
        assert_eq!(texts.len(), 10, "100 events become 10 digest messages");
        assert!(texts[0].contains("10 event(s)"), "{}", texts[0]);

        // Below the threshold the window decides.
        for seq in 101..=105 {
            enqueue_at(&mut store, &event(seq), "", rules(&[], &telegram_rules), 0).expect("queue");
        }
        let report =
            process(&mut store, 10, policy, &mut |_, _| SendOutcome::ok(200)).expect("process");
        assert_eq!(report.sent, 0, "5 events and 10 s are not enough");
        let report =
            process(&mut store, 31, policy, &mut |_, _| SendOutcome::ok(200)).expect("process");
        assert_eq!(report.sent, 5, "the 30 s window flushes them");
    }

    #[test]
    fn a_thousand_events_drain_within_a_minute() {
        let mut store = MemoryStore::new();
        let telegram_rules = vec![telegram("42")];
        for seq in 1..=1_000 {
            enqueue_at(&mut store, &event(seq), "", rules(&[], &telegram_rules), 0).expect("queue");
        }
        let started = std::time::Instant::now();
        let policy = DigestPolicy {
            events: 10,
            window_secs: 30,
            ..DigestPolicy::default()
        };
        let mut sent = 0;
        let report = process(&mut store, 0, policy, &mut |_, _| {
            sent += 1;
            SendOutcome::ok(200)
        })
        .expect("process");
        assert_eq!(report.sent, 1_000);
        assert_eq!(sent, 100, "1000 events become 100 digest messages");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn telegram_text_is_escaped_and_truncated() {
        let mut delivery = Delivery {
            seq: 1,
            event_seq: 1,
            event: EventKind::Finished,
            torrent_id: 1,
            name: "<movie> & more".to_owned(),
            infohash: String::new(),
            label: String::new(),
            url: String::new(),
            secret: String::new(),
            telegram: Some(TelegramTarget {
                api_base: TELEGRAM_API_BASE.to_owned(),
                bot_token: "t".to_owned(),
                chat_id: "1".to_owned(),
                message_thread_id: String::new(),
            }),
            timeout_ms: 1000,
            attempt: 0,
            next_at: 0,
            dead: false,
            last_status: None,
            last_error: None,
            created_at: 1,
        };
        let text = delivery.telegram_text();
        assert!(text.contains("&lt;movie&gt; &amp; more"), "{text}");
        delivery.name = "x".repeat(5_000);
        assert_eq!(delivery.telegram_text().chars().count(), TELEGRAM_MAX_CHARS);
        assert!(delivery.telegram_text().ends_with('…'));
    }

    #[test]
    fn quiet_bounds_parse_and_validate() {
        assert_eq!(parse_quiet("", "").expect("off"), None);
        assert_eq!(
            parse_quiet("23:00", "07:00").expect("parse"),
            Some((1380, 420))
        );
        assert!(parse_quiet("24:00", "07:00").is_err());
        assert!(parse_quiet("23:00", "7").is_err());
    }
}
