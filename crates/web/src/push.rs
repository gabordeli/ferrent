//! Web Push: the VAPID identity, the subscription store and the sender.
//!
//! Browsers subscribe with the public key of the gateway's VAPID pair; the
//! store keeps the endpoint and the two client keys. The daemon's `Watch`
//! stream feeds the tracker, which turns a torrent finishing (or entering the
//! error state) into one push per subscription.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use ferrent_protocol::v1::TorrentState;
use ferrent_protocol::v1::WatchResponse;
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushError, WebPushMessageBuilder,
};

use crate::AppState;
use crate::Session;
use crate::config::PushConfig;

/// One browser subscription, as stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSubscription {
    pub id: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub host: String,
    pub user_agent: String,
    pub added: u64,
    pub last_seen: u64,
}

/// One row of the device list in the UI.
#[derive(Debug, Serialize)]
pub struct SubscriptionView {
    pub id: String,
    pub host: String,
    pub added: u64,
    pub last_seen: u64,
    pub user_agent: String,
}

/// The browser's `PushSubscription.toJSON()` body.
#[derive(Debug, Deserialize)]
pub struct SubscriptionRequest {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
}

/// The two client keys of a subscription.
#[derive(Debug, Deserialize)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Deserialize)]
pub struct EndpointRequest {
    pub endpoint: String,
}

/// The VAPID identity, the store and the HTTP sender.
pub struct PushState {
    config: PushConfig,
    pem: String,
    public_key: String,
    subscriptions: Mutex<Vec<StoredSubscription>>,
    tracker: Mutex<Tracker>,
    client: HyperWebPushClient,
}

/// What the daemon stream says about one torrent.
#[derive(Debug, Default)]
struct Tracker {
    names: HashMap<u64, String>,
    errors: HashMap<u64, String>,
    states: HashMap<u64, i32>,
    finished: HashSet<u64>,
}

/// One notification to deliver.
#[derive(Debug, PartialEq, Eq)]
struct Notification {
    kind: &'static str,
    name: String,
    message: String,
    tag: String,
}

impl Tracker {
    /// Feeds one delta; returns a notification when it is worth sending.
    fn observe(&mut self, response: &WatchResponse) -> Option<Notification> {
        match response.event.as_ref()? {
            WatchEvent::Added(torrent) => {
                self.names.insert(torrent.id, torrent.name.clone());
                self.errors.insert(torrent.id, torrent.error.clone());
                self.states.insert(torrent.id, torrent.state);
                self.finished.remove(&torrent.id);
                None
            }
            WatchEvent::StateChanged(change) => {
                let previous = self.states.insert(change.id, change.state);
                // The initial snapshot must not notify: only a transition into
                // the error state does.
                let entered_error = change.state == TorrentState::Error as i32
                    && previous.is_some_and(|state| state != change.state);
                entered_error
                    .then(|| self.error_notification(change.id))
                    .flatten()
            }
            WatchEvent::Finished(finished) => {
                if self.finished.insert(finished.id) {
                    Some(Notification {
                        kind: "finished",
                        name: self.name_of(finished.id),
                        message: String::new(),
                        tag: format!("torrent-{}", finished.id),
                    })
                } else {
                    None
                }
            }
            WatchEvent::Removed(removed) => {
                self.names.remove(&removed.id);
                self.errors.remove(&removed.id);
                self.states.remove(&removed.id);
                self.finished.remove(&removed.id);
                None
            }
            _ => None,
        }
    }

    fn error_notification(&self, id: u64) -> Option<Notification> {
        let name = self.names.get(&id)?.clone();
        Some(Notification {
            kind: "error",
            name,
            message: self.errors.get(&id).cloned().unwrap_or_default(),
            tag: format!("torrent-{id}"),
        })
    }

    fn name_of(&self, id: u64) -> String {
        self.names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("#{id}"))
    }
}

impl PushState {
    /// Loads the VAPID key (generating it when missing) and the store.
    pub fn load(config: &PushConfig) -> Result<Self, String> {
        let pem = load_or_generate_pem(&config.private_key)?;
        let public_key = public_key_of(&pem)?;
        let subscriptions = load_store(&config.subscriptions)?;
        Ok(Self {
            config: config.clone(),
            pem,
            public_key,
            subscriptions: Mutex::new(subscriptions),
            tracker: Mutex::new(Tracker::default()),
            client: HyperWebPushClient::new(),
        })
    }

    /// The base64url public key browsers subscribe with.
    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// How many browsers are subscribed.
    #[must_use]
    pub fn count(&self) -> usize {
        self.subscriptions
            .lock()
            .expect("push store poisoned")
            .len()
    }

    /// The device list, without the client keys.
    #[must_use]
    pub fn list(&self) -> Vec<SubscriptionView> {
        self.subscriptions
            .lock()
            .expect("push store poisoned")
            .iter()
            .map(|subscription| SubscriptionView {
                id: subscription.id.clone(),
                host: subscription.host.clone(),
                added: subscription.added,
                last_seen: subscription.last_seen,
                user_agent: subscription.user_agent.clone(),
            })
            .collect()
    }

    /// Adds or refreshes one subscription; returns the new count.
    pub fn subscribe(
        &self,
        request: SubscriptionRequest,
        user_agent: &str,
    ) -> Result<usize, String> {
        if !request.endpoint.starts_with("https://") {
            return Err("endpoint must be an https URL".to_owned());
        }
        let p256dh = decode_url(&request.keys.p256dh)
            .ok_or_else(|| "keys.p256dh is not base64url".to_owned())?;
        let auth = decode_url(&request.keys.auth)
            .ok_or_else(|| "keys.auth is not base64url".to_owned())?;
        if p256dh.len() != 65 {
            return Err("keys.p256dh must be an uncompressed P-256 point".to_owned());
        }
        if auth.len() != 16 {
            return Err("keys.auth must be 16 bytes".to_owned());
        }
        let now = unix_now();
        let mut subscriptions = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .clone();
        match subscriptions
            .iter_mut()
            .find(|subscription| subscription.endpoint == request.endpoint)
        {
            Some(existing) => {
                existing.p256dh = request.keys.p256dh;
                existing.auth = request.keys.auth;
                existing.user_agent = user_agent.to_owned();
                existing.last_seen = now;
            }
            None => subscriptions.push(StoredSubscription {
                id: id_of(&request.endpoint),
                host: host_of(&request.endpoint),
                endpoint: request.endpoint,
                p256dh: request.keys.p256dh,
                auth: request.keys.auth,
                user_agent: user_agent.to_owned(),
                added: now,
                last_seen: now,
            }),
        }
        self.commit(subscriptions)
    }

    /// Drops the subscription of one endpoint.
    pub fn unsubscribe(&self, endpoint: &str) -> bool {
        let subscriptions = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .clone();
        let kept: Vec<StoredSubscription> = subscriptions
            .into_iter()
            .filter(|subscription| subscription.endpoint != endpoint)
            .collect();
        let removed = kept.len() != self.count();
        if removed {
            let _ = self.commit(kept);
        }
        removed
    }

    /// Drops one subscription by its id.
    pub fn remove(&self, id: &str) -> bool {
        let subscriptions = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .clone();
        let kept: Vec<StoredSubscription> = subscriptions
            .into_iter()
            .filter(|subscription| subscription.id != id)
            .collect();
        let removed = kept.len() != self.count();
        if removed {
            let _ = self.commit(kept);
        }
        removed
    }

    /// Feeds one daemon delta and fans a notification out to the subscribers.
    pub fn observe(self: &Arc<Self>, response: &WatchResponse) {
        if self.count() == 0 {
            return;
        }
        let notification = {
            let mut tracker = self.tracker.lock().expect("push tracker poisoned");
            tracker.observe(response)
        };
        if let Some(notification) = notification {
            self.notify(notification);
        }
    }

    /// Sends the test notification to one endpoint.
    pub fn send_test(self: &Arc<Self>, endpoint: &str) -> bool {
        let subscription = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .iter()
            .find(|subscription| subscription.endpoint == endpoint)
            .cloned();
        let Some(subscription) = subscription else {
            return false;
        };
        self.send(
            subscription,
            payload(&Notification {
                kind: "test",
                name: String::new(),
                message: String::new(),
                tag: "test".to_owned(),
            }),
        );
        true
    }

    fn notify(self: &Arc<Self>, notification: Notification) {
        let payload = payload(&notification);
        let subscriptions = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .clone();
        for subscription in subscriptions {
            self.send(subscription, payload.clone());
        }
    }

    fn send(self: &Arc<Self>, subscription: StoredSubscription, payload: Vec<u8>) {
        let client = self.client.clone();
        let pem = self.pem.clone();
        let subject = self.config.subject.clone();
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let info = SubscriptionInfo::new(
                &subscription.endpoint,
                &subscription.p256dh,
                &subscription.auth,
            );
            let mut builder = WebPushMessageBuilder::new(&info);
            builder.set_payload(ContentEncoding::Aes128Gcm, &payload);
            builder.set_ttl(600);
            let Ok(mut vapid) = VapidSignatureBuilder::from_pem(pem.as_bytes(), &info) else {
                return;
            };
            vapid.add_claim("sub", subject);
            let Ok(signature) = vapid.build() else {
                return;
            };
            builder.set_vapid_signature(signature);
            let Ok(message) = builder.build() else {
                return;
            };
            match client.send(message).await {
                Ok(()) => state.touch(&subscription.endpoint),
                Err(WebPushError::EndpointNotValid(_) | WebPushError::EndpointNotFound(_)) => {
                    let _ = state.unsubscribe(&subscription.endpoint);
                }
                Err(_) => {}
            }
        });
    }

    /// Refreshes `last_seen` in memory only; the store is not rewritten per
    /// notification.
    fn touch(&self, endpoint: &str) {
        if let Some(subscription) = self
            .subscriptions
            .lock()
            .expect("push store poisoned")
            .iter_mut()
            .find(|subscription| subscription.endpoint == endpoint)
        {
            subscription.last_seen = unix_now();
        }
    }

    fn commit(&self, subscriptions: Vec<StoredSubscription>) -> Result<usize, String> {
        let text = serde_json::to_string_pretty(&subscriptions)
            .map_err(|error| format!("cannot encode the subscription store: {error}"))?;
        ferrent_platform::write_secret(&self.config.subscriptions, text.as_bytes())
            .map_err(|error| format!("cannot write the subscription store: {error}"))?;
        let count = subscriptions.len();
        *self.subscriptions.lock().expect("push store poisoned") = subscriptions;
        Ok(count)
    }
}

fn payload(notification: &Notification) -> Vec<u8> {
    let value = match notification.kind {
        "test" => json!({ "kind": "test", "url": "/" }),
        "finished" => json!({
            "kind": "finished",
            "name": &notification.name,
            "url": "/",
            "tag": &notification.tag,
        }),
        _ => json!({
            "kind": "error",
            "name": &notification.name,
            "message": &notification.message,
            "url": "/",
            "tag": &notification.tag,
        }),
    };
    value.to_string().into_bytes()
}

fn load_or_generate_pem(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let pem = generate_pem()?;
            ferrent_platform::write_secret(path, pem.as_bytes())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            Ok(pem)
        }
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn generate_pem() -> Result<String, String> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| "cannot generate the VAPID key".to_owned())?;
    Ok(pem_encode("PRIVATE KEY", pkcs8.as_ref()))
}

fn pem_encode(label: &str, der: &[u8]) -> String {
    let encoded = STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn public_key_of(pem: &str) -> Result<String, String> {
    let partial = VapidSignatureBuilder::from_pem_no_sub(pem.as_bytes())
        .map_err(|error| format!("invalid VAPID key: {error:?}"))?;
    Ok(URL_SAFE_NO_PAD.encode(partial.get_public_key()))
}

fn load_store(path: &Path) -> Result<Vec<StoredSubscription>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("invalid {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn decode_url(value: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| URL_SAFE.decode(value))
        .ok()
}

fn id_of(endpoint: &str) -> String {
    let digest = Sha256::digest(endpoint.as_bytes());
    digest.iter().take(8).fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    })
}

fn host_of(endpoint: &str) -> String {
    endpoint
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or_default()
        .to_owned()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn disabled() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "push is disabled" })),
    )
        .into_response()
}

/// `GET /api/push/config`: whether push is on and the key to subscribe with.
pub async fn config(State(state): State<Arc<AppState>>, _session: Session) -> Response {
    match state.push.as_ref() {
        Some(push) => Json(json!({
            "enabled": true,
            "public_key": push.public_key(),
            "subscriptions": push.count(),
        }))
        .into_response(),
        None => Json(json!({
            "enabled": false,
            "public_key": "",
            "subscriptions": 0,
        }))
        .into_response(),
    }
}

/// `POST /api/push/subscribe`: the browser's subscription JSON.
pub async fn subscribe(
    State(state): State<Arc<AppState>>,
    _session: Session,
    headers: HeaderMap,
    Json(request): Json<SubscriptionRequest>,
) -> Response {
    let Some(push) = state.push.as_ref() else {
        return disabled();
    };
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    match push.subscribe(request, user_agent) {
        Ok(count) => Json(json!({ "count": count })).into_response(),
        Err(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
        }
    }
}

/// `DELETE /api/push/subscribe`: forget one endpoint.
pub async fn unsubscribe(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Json(request): Json<EndpointRequest>,
) -> Response {
    let Some(push) = state.push.as_ref() else {
        return disabled();
    };
    Json(json!({ "unsubscribed": push.unsubscribe(&request.endpoint) })).into_response()
}

/// `GET /api/push/subscriptions`: the device list.
pub async fn subscriptions(State(state): State<Arc<AppState>>, _session: Session) -> Response {
    let Some(push) = state.push.as_ref() else {
        return Json(json!({ "subscriptions": [] })).into_response();
    };
    Json(json!({ "subscriptions": push.list() })).into_response()
}

/// `DELETE /api/push/subscriptions/{id}`: remove one device.
pub async fn remove(
    State(state): State<Arc<AppState>>,
    _session: Session,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let Some(push) = state.push.as_ref() else {
        return disabled();
    };
    Json(json!({ "unsubscribed": push.remove(&id) })).into_response()
}

/// `POST /api/push/test`: send one test notification to an endpoint.
pub async fn test(
    State(state): State<Arc<AppState>>,
    _session: Session,
    Json(request): Json<EndpointRequest>,
) -> Response {
    let Some(push) = state.push.as_ref() else {
        return disabled();
    };
    if push.send_test(&request.endpoint) {
        Json(json!({ "queued": true })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown endpoint" })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrent_protocol::v1::{Removed, StateChanged, Torrent, TorrentFinished};

    fn response(event: WatchEvent) -> WatchResponse {
        WatchResponse { event: Some(event) }
    }

    fn torrent(id: u64, state: i32) -> Torrent {
        Torrent {
            id,
            name: format!("t{id}"),
            state,
            error: String::new(),
            ..Torrent::default()
        }
    }

    #[test]
    fn a_finished_torrent_notifies_once_with_its_name() {
        let mut tracker = Tracker::default();
        tracker.observe(&response(WatchEvent::Added(torrent(4, 3))));
        let first = tracker
            .observe(&response(WatchEvent::Finished(TorrentFinished { id: 4 })))
            .expect("finished notification");
        assert_eq!(first.kind, "finished");
        assert_eq!(first.name, "t4");
        assert_eq!(first.tag, "torrent-4");
        assert!(
            tracker
                .observe(&response(WatchEvent::Finished(TorrentFinished { id: 4 })))
                .is_none(),
            "the same finish is not sent twice"
        );
    }

    #[test]
    fn a_removed_and_readded_torrent_can_finish_again() {
        let mut tracker = Tracker::default();
        tracker.observe(&response(WatchEvent::Added(torrent(4, 3))));
        tracker.observe(&response(WatchEvent::Finished(TorrentFinished { id: 4 })));
        tracker.observe(&response(WatchEvent::Removed(Removed { id: 4 })));
        tracker.observe(&response(WatchEvent::Added(torrent(4, 3))));
        assert!(
            tracker
                .observe(&response(WatchEvent::Finished(TorrentFinished { id: 4 })))
                .is_some()
        );
    }

    #[test]
    fn only_a_transition_into_error_notifies() {
        let mut tracker = Tracker::default();
        let error = TorrentState::Error as i32;
        tracker.observe(&response(WatchEvent::Added(Torrent {
            id: 5,
            name: "bad".to_owned(),
            state: 3,
            error: "disk full".to_owned(),
            ..Torrent::default()
        })));
        assert!(
            tracker
                .observe(&response(WatchEvent::StateChanged(StateChanged {
                    id: 5,
                    state: error,
                })))
                .is_some(),
            "downloading -> error"
        );
        assert!(
            tracker
                .observe(&response(WatchEvent::StateChanged(StateChanged {
                    id: 5,
                    state: error,
                })))
                .is_none(),
            "the same error is not repeated"
        );
    }

    #[test]
    fn the_initial_snapshot_does_not_notify() {
        let mut tracker = Tracker::default();
        let error = TorrentState::Error as i32;
        tracker.observe(&response(WatchEvent::Added(torrent(6, error))));
        assert!(
            tracker
                .observe(&response(WatchEvent::StateChanged(StateChanged {
                    id: 6,
                    state: error,
                })))
                .is_none(),
            "an already failing torrent at connect time stays quiet"
        );
    }

    #[test]
    fn the_vapid_key_round_trips_through_a_private_file() {
        let dir = std::env::temp_dir().join(format!("ferrent-push-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("vapid.pem");

        let pem = load_or_generate_pem(&path).expect("generate");
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        let again = load_or_generate_pem(&path).expect("reuse");
        assert_eq!(pem, again, "the key is not regenerated");

        let public = public_key_of(&pem).expect("public key");
        let bytes = decode_url(&public).expect("base64url");
        assert_eq!(bytes.len(), 65, "uncompressed P-256 point");
        assert_eq!(bytes[0], 4);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_store_adds_refreshes_and_drops_subscriptions() {
        let dir = std::env::temp_dir().join(format!("ferrent-push-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let config = PushConfig {
            enabled: true,
            subject: "mailto:test@example.com".to_owned(),
            private_key: dir.join("vapid.pem"),
            subscriptions: dir.join("subs.json"),
        };
        let push = PushState::load(&config).expect("state");
        let request = |endpoint: &str| SubscriptionRequest {
            endpoint: endpoint.to_owned(),
            keys: SubscriptionKeys {
                p256dh: URL_SAFE_NO_PAD.encode([4_u8; 65]),
                auth: URL_SAFE_NO_PAD.encode([1_u8; 16]),
            },
        };

        assert_eq!(
            push.subscribe(request("https://push.example/1"), "UA/1.0")
                .expect("subscribe"),
            1
        );
        assert_eq!(push.list().len(), 1);
        assert_eq!(
            push.subscribe(request("https://push.example/1"), "UA/2.0")
                .expect("refresh"),
            1,
            "the same endpoint is refreshed, not duplicated"
        );
        assert_eq!(push.list()[0].user_agent, "UA/2.0");

        let id = push.list()[0].id.clone();
        assert!(push.remove(&id), "removed by id");
        assert_eq!(push.count(), 0);

        push.subscribe(request("https://push.example/2"), "UA")
            .expect("subscribe again");
        assert!(
            push.unsubscribe("https://push.example/2"),
            "removed by endpoint"
        );
        assert_eq!(push.count(), 0);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_subscriptions_are_rejected() {
        let dir = std::env::temp_dir().join(format!("ferrent-push-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = PushConfig {
            enabled: true,
            subject: "mailto:test@example.com".to_owned(),
            private_key: dir.join("vapid.pem"),
            subscriptions: dir.join("subs.json"),
        };
        let push = PushState::load(&config).expect("state");
        let keys = || SubscriptionKeys {
            p256dh: URL_SAFE_NO_PAD.encode([4_u8; 65]),
            auth: URL_SAFE_NO_PAD.encode([1_u8; 16]),
        };
        let mut request = SubscriptionRequest {
            endpoint: "http://insecure.example/1".to_owned(),
            keys: keys(),
        };
        assert!(push.subscribe(request, "UA").is_err(), "plain http");

        request = SubscriptionRequest {
            endpoint: "https://push.example/1".to_owned(),
            keys: SubscriptionKeys {
                p256dh: URL_SAFE_NO_PAD.encode([4_u8; 65]),
                auth: URL_SAFE_NO_PAD.encode([1_u8; 8]),
            },
        };
        assert!(push.subscribe(request, "UA").is_err(), "short auth");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hosts_and_ids_are_derived_from_the_endpoint() {
        assert_eq!(
            host_of("https://fcm.googleapis.com/wp/abc"),
            "fcm.googleapis.com"
        );
        assert_eq!(id_of("https://a"), id_of("https://a"));
        assert_ne!(id_of("https://a"), id_of("https://b"));
        assert_eq!(id_of("https://a").len(), 16);
    }
}
