//! SSE bridge: one daemon subscription, coalesced into at most one message
//! per window per client.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ferrent_protocol::v1::{Torrent, WatchRequest, WatchResponse};
use serde_json::{Map, Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::AppState;

/// One coalesced batch.
#[derive(Debug, Default)]
pub struct Batch {
    changed: BTreeMap<u64, Map<String, Value>>,
    removed: BTreeSet<u64>,
}

impl Batch {
    /// Applies one daemon delta.
    pub fn apply(&mut self, response: &WatchResponse) {
        match response.event.as_ref() {
            Some(WatchEvent::Added(torrent)) => {
                self.removed.remove(&torrent.id);
                self.changed.insert(torrent.id, torrent_json(torrent));
            }
            Some(WatchEvent::StateChanged(change)) => {
                self.patch(change.id)
                    .insert("state".to_owned(), json!(change.state));
            }
            Some(WatchEvent::Progress(progress)) => {
                let patch = self.patch(progress.id);
                patch.insert("progress".to_owned(), json!(progress.progress));
                patch.insert("download_rate".to_owned(), json!(progress.download_rate));
                patch.insert("upload_rate".to_owned(), json!(progress.upload_rate));
                patch.insert("queue_position".to_owned(), json!(progress.queue_position));
            }
            Some(WatchEvent::Removed(removed)) => {
                self.changed.remove(&removed.id);
                self.removed.insert(removed.id);
            }
            Some(WatchEvent::ConfigChanged(_) | WatchEvent::Finished(_)) | None => {}
        }
    }

    /// Whether the batch has anything to send.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }

    /// Renders the batch and clears it.
    pub fn take(&mut self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let changed: Vec<&Map<String, Value>> = self.changed.values().collect();
        let removed: Vec<u64> = self.removed.iter().copied().collect();
        let payload = json!({ "changed": changed, "removed": removed }).to_string();
        self.changed.clear();
        self.removed.clear();
        Some(payload)
    }

    fn patch(&mut self, id: u64) -> &mut Map<String, Value> {
        // A torrent that comes back starts from a fresh object.
        self.removed.remove(&id);
        self.changed.entry(id).or_insert_with(|| {
            let mut map = Map::new();
            map.insert("id".to_owned(), json!(id));
            map
        })
    }
}

/// The JSON shape of one torrent in a delta.
#[must_use]
pub fn torrent_json(torrent: &Torrent) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("id".to_owned(), json!(torrent.id));
    map.insert("name".to_owned(), json!(torrent.name));
    map.insert("total_bytes".to_owned(), json!(torrent.total_bytes));
    map.insert("state".to_owned(), json!(torrent.state));
    map.insert("progress".to_owned(), json!(torrent.progress));
    map.insert("download_rate".to_owned(), json!(torrent.download_rate));
    map.insert("upload_rate".to_owned(), json!(torrent.upload_rate));
    map.insert("seeds".to_owned(), json!(torrent.seeds));
    map.insert("peers".to_owned(), json!(torrent.peers));
    map.insert("eta_seconds".to_owned(), json!(torrent.eta_seconds));
    map.insert("save_path".to_owned(), json!(torrent.save_path));
    map.insert("label".to_owned(), json!(torrent.label));
    map.insert("error".to_owned(), json!(torrent.error));
    map.insert("queue_position".to_owned(), json!(torrent.queue_position));
    map.insert("infohash".to_owned(), json!(torrent.infohash));
    map.insert("pause_reasons".to_owned(), json!(torrent.pause_reasons));
    map.insert("sequential".to_owned(), json!(torrent.sequential));
    map.insert("super_seeding".to_owned(), json!(torrent.super_seeding));
    map
}

/// Starts the daemon subscription and the coalescing loop.
///
/// The loop reconnects with a one-second delay; `/health` follows the
/// connection flag.
pub fn spawn_watch(state: Arc<AppState>) {
    tokio::spawn(async move {
        let window = Duration::from_millis(state.config.coalesce_ms);
        loop {
            let ca = match std::fs::read_to_string(&state.config.daemon.ca) {
                Ok(pem) => pem,
                Err(error) => {
                    eprintln!("web: cannot read CA: {error}");
                    state.daemon_up.store(false, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let client = match ferrent_protocol::client::connect(
                &state.config.daemon.url,
                &ca,
                &state.config.daemon.token,
            )
            .await
            {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("web: daemon connect failed: {error}");
                    state.daemon_up.store(false, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            *state.client.lock().expect("daemon client poisoned") = Some(client.clone());
            state.daemon_up.store(true, Ordering::Relaxed);
            let mut stream = match client.clone().watch(WatchRequest {}).await {
                Ok(response) => response.into_inner(),
                Err(error) => {
                    eprintln!("web: watch failed: {error}");
                    state.daemon_up.store(false, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let mut batch = Batch::default();
            let mut tick = tokio::time::interval(window);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    message = stream.message() => match message {
                        Ok(Some(response)) => {
                            batch.apply(&response);
                            if let Some(push) = state.push.as_ref() {
                                push.observe(&response);
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            eprintln!("web: watch stream ended: {error}");
                            break;
                        }
                    },
                    _ = tick.tick() => {
                        if let Some(payload) = batch.take() {
                            let _ = state.hub.send(payload);
                        }
                    }
                }
            }
            state.daemon_up.store(false, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// The SSE response of one browser.
pub fn stream(
    state: Arc<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let receiver = state.hub.subscribe();
    let heartbeat = state.config.heartbeat_secs;
    let stream = BroadcastStream::new(receiver).filter_map(|message| match message {
        Ok(payload) => Some(Ok(Event::default().event("deltas").data(payload))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(heartbeat))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn torrent(id: u64) -> Torrent {
        Torrent {
            id,
            name: format!("t{id}"),
            total_bytes: 100,
            state: 2,
            progress: 0.5,
            download_rate: 10,
            upload_rate: 1,
            error: String::new(),
            seeds: 1,
            peers: 2,
            eta_seconds: 5,
            save_path: "/data".to_owned(),
            label: "movies".to_owned(),
            ..Torrent::default()
        }
    }

    fn response(event: WatchEvent) -> WatchResponse {
        WatchResponse { event: Some(event) }
    }

    #[test]
    fn progress_merges_into_one_object_per_torrent() {
        let mut batch = Batch::default();
        batch.apply(&response(WatchEvent::Added(torrent(7))));
        batch.apply(&response(WatchEvent::Progress(
            ferrent_protocol::v1::Progress {
                id: 7,
                progress: 0.9,
                download_rate: 100,
                upload_rate: 2,
                queue_position: 3,
            },
        )));
        batch.apply(&response(WatchEvent::StateChanged(
            ferrent_protocol::v1::StateChanged { id: 7, state: 4 },
        )));
        let payload = batch.take().expect("payload");
        let value: Value = serde_json::from_str(&payload).expect("json");
        assert_eq!(value["changed"].as_array().expect("changed").len(), 1);
        let entry = &value["changed"][0];
        assert_eq!(entry["id"], 7);
        assert_eq!(entry["name"], "t7");
        let progress = entry["progress"].as_f64().expect("progress");
        assert!((progress - 0.9).abs() < 1e-6, "{progress}");
        assert_eq!(entry["state"], 4);
        assert!(batch.take().is_none(), "the batch is cleared");
    }

    #[test]
    fn an_empty_batch_sends_nothing() {
        let mut batch = Batch::default();
        batch.apply(&response(WatchEvent::ConfigChanged(
            ferrent_protocol::v1::ConfigChanged { revision: 2 },
        )));
        assert!(batch.is_empty());
        assert!(batch.take().is_none());
    }

    #[test]
    fn removal_wins_over_a_later_patch() {
        let mut batch = Batch::default();
        batch.apply(&response(WatchEvent::Removed(
            ferrent_protocol::v1::Removed { id: 9 },
        )));
        let payload = batch.take().expect("payload");
        let value: Value = serde_json::from_str(&payload).expect("json");
        assert_eq!(value["removed"], json!([9]));
        assert_eq!(value["changed"].as_array().expect("changed").len(), 0);
    }
}
