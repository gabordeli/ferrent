//! The daemon side of the RSS watcher: fetch each feed on a schedule, add the
//! new `.torrent` items once, and remember what was already seen.
//!
//! State lives in the same redb store as the torrents:
//! `rss-seen:<feed>:<key>` (dedupe), `rss-etag:`/`rss-modified:` (conditional
//! GET), `rss-last:` (last poll, unix seconds), `rss-error:` and `rss-added:`.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ferrent_engine::{AddTorrent, EngineError, TorrentSource};
use ferrent_protocol::v1::RssFeed as ProtoFeed;

use crate::persist::{self, PersistedTorrent};
use crate::rpc::AppState;
use crate::rss;
use crate::store::StoreOp;

/// One feed as reported to the client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedStatus {
    pub url: String,
    pub label: String,
    pub last_poll: u64,
    pub error: String,
    pub added: u64,
    pub seen: u64,
}

impl FeedStatus {
    #[must_use]
    pub fn to_proto(&self) -> ProtoFeed {
        ProtoFeed {
            url: self.url.clone(),
            label: self.label.clone(),
            last_poll: self.last_poll,
            error: self.error.clone(),
            added: self.added,
            seen: self.seen,
        }
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn seen_key(feed: &str, key: &str) -> String {
    format!("rss-seen:{feed}:{key}")
}

fn meta_key(name: &str, feed: &str) -> String {
    format!("rss-{name}:{feed}")
}

/// One item of a previewed feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewItem {
    pub title: String,
    /// The URL the watcher would download; empty when the item has none.
    pub torrent_url: String,
    /// The item is a torrent and passes the title filter.
    pub matched: bool,
    /// Empty when matched, otherwise `not a torrent` or `filter`.
    pub reason: String,
}

/// A preview fetch: every item, and whether the watcher would add it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewReport {
    pub total: u32,
    pub items: Vec<PreviewItem>,
}

/// Classifies parsed items against the title filter.
///
/// Mirrors `poll_feed`: a candidate needs a torrent/magnet URL first, then the
/// filter runs on the title. Pure, so it is testable without a network.
#[must_use]
pub fn classify(items: Vec<rss::Item>, filter: Option<&regex::Regex>) -> PreviewReport {
    let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let items = items
        .into_iter()
        .map(|item| {
            let torrent_url = item.torrent_url().trim().to_owned();
            if !rss::looks_like_torrent(&torrent_url) {
                PreviewItem {
                    title: item.title,
                    torrent_url,
                    matched: false,
                    reason: "not a torrent".to_owned(),
                }
            } else if filter.is_some_and(|filter| !filter.is_match(&item.title)) {
                PreviewItem {
                    title: item.title,
                    torrent_url,
                    matched: false,
                    reason: "filter".to_owned(),
                }
            } else {
                PreviewItem {
                    title: item.title,
                    torrent_url,
                    matched: true,
                    reason: String::new(),
                }
            }
        })
        .collect();
    PreviewReport { total, items }
}

impl AppState {
    /// Fetches one feed and reports what the watcher would add from it.
    pub async fn rss_preview(&self, url: &str, filter: &str) -> Result<PreviewReport, String> {
        let url = url.trim().to_owned();
        if url.is_empty() {
            return Err("feed URL is empty".to_owned());
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("feed URL must start with http:// or https://".to_owned());
        }
        let regex = if filter.trim().is_empty() {
            None
        } else {
            Some(regex::Regex::new(filter.trim()).map_err(|error| format!("filter: {error}"))?)
        };
        let target = url.clone();
        let fetched = tokio::task::spawn_blocking(move || {
            rss::fetch(&target, None, None, Duration::from_secs(20))
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("fetch: {error}"))?;
        let items = rss::parse(&fetched.body)?;
        Ok(classify(items, regex.as_ref()))
    }
}

impl AppState {
    /// Reads the stored status of every configured feed.
    pub fn rss_status(&self) -> Vec<FeedStatus> {
        let Ok(config) = self.config.resolved() else {
            return Vec::new();
        };
        let store = self.store.lock().expect("state store mutex poisoned");
        config
            .rss
            .feed
            .iter()
            .filter(|feed| feed.enabled)
            .map(|feed| {
                let text = |name: &str| {
                    store
                        .get(&meta_key(name, feed.url.trim()))
                        .ok()
                        .flatten()
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .unwrap_or_default()
                };
                FeedStatus {
                    url: feed.url.trim().to_owned(),
                    label: feed.label.clone(),
                    last_poll: text("last").parse().unwrap_or(0),
                    error: text("error"),
                    added: text("added").parse().unwrap_or(0),
                    seen: store
                        .list(&format!("rss-seen:{}:", feed.url.trim()))
                        .map(|entries| entries.len() as u64)
                        .unwrap_or(0),
                }
            })
            .collect()
    }

    /// One polling pass over every enabled feed. Errors stay per feed.
    pub async fn rss_tick(&self) -> Vec<FeedStatus> {
        let Ok(config) = self.config.resolved() else {
            return Vec::new();
        };
        if !config.rss.enabled {
            return Vec::new();
        }
        let mut statuses = Vec::new();
        for feed in config.rss.feed.iter().filter(|feed| feed.enabled) {
            let url = feed.url.trim().to_owned();
            let mut status = FeedStatus {
                url: url.clone(),
                label: feed.label.clone(),
                ..FeedStatus::default()
            };
            // The counter is cumulative across restarts.
            status.added = self
                .store
                .lock()
                .ok()
                .and_then(|store| store.get(&meta_key("added", &url)).ok().flatten())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|text| text.parse().ok())
                .unwrap_or(0);
            match self.poll_feed(feed, &config, &mut status).await {
                Ok(()) => {
                    status.error.clear();
                }
                Err(error) => status.error = error,
            }
            status.last_poll = now_epoch();
            let store_ops = vec![
                StoreOp::Put {
                    key: meta_key("last", &url),
                    value: status.last_poll.to_string().into_bytes(),
                },
                StoreOp::Put {
                    key: meta_key("error", &url),
                    value: status.error.clone().into_bytes(),
                },
                StoreOp::Put {
                    key: meta_key("added", &url),
                    value: status.added.to_string().into_bytes(),
                },
            ];
            if let Ok(mut store) = self.store.lock() {
                let _ = store.batch(store_ops);
            }
            statuses.push(status);
        }
        statuses
    }

    async fn poll_feed(
        &self,
        feed: &crate::config::RssFeedConfig,
        config: &crate::config::ResolvedConfig,
        status: &mut FeedStatus,
    ) -> Result<(), String> {
        let url = feed.url.trim().to_owned();
        let (etag, modified) = {
            let store = self.store.lock().map_err(|_| "store poisoned".to_owned())?;
            let read = |name: &str| {
                store
                    .get(&meta_key(name, &url))
                    .ok()
                    .flatten()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .filter(|value| !value.is_empty())
            };
            (read("etag"), read("modified"))
        };

        let target = url.clone();
        let fetched = tokio::task::spawn_blocking(move || {
            rss::fetch(
                &target,
                etag.as_deref(),
                modified.as_deref(),
                Duration::from_secs(20),
            )
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("fetch: {error}"))?;

        if fetched.not_modified {
            return Ok(());
        }
        let items = rss::parse(&fetched.body)?;
        let filter = if feed.filter.trim().is_empty() {
            None
        } else {
            Some(regex::Regex::new(feed.filter.trim()).map_err(|error| error.to_string())?)
        };

        for item in items {
            let torrent_url = item.torrent_url().trim().to_owned();
            if !rss::looks_like_torrent(&torrent_url) {
                continue;
            }
            if let Some(filter) = &filter {
                if !filter.is_match(&item.title) {
                    continue;
                }
            }
            let key = seen_key(&url, &item.key());
            {
                let store = self.store.lock().map_err(|_| "store poisoned".to_owned())?;
                if store.get(&key).ok().flatten().is_some() {
                    continue;
                }
            }
            match self.add_rss_item(feed, config, &item, &torrent_url).await {
                Ok(id) => {
                    status.added += 1;
                    if let Ok(mut store) = self.store.lock() {
                        let _ = store.batch(vec![StoreOp::Put {
                            key,
                            value: id.to_string().into_bytes(),
                        }]);
                    }
                }
                Err(EngineError::AlreadyExists { id }) => {
                    if let Ok(mut store) = self.store.lock() {
                        let _ = store.batch(vec![StoreOp::Put {
                            key,
                            value: id.to_string().into_bytes(),
                        }]);
                    }
                }
                Err(EngineError::Rejected { reason }) => {
                    status.error = format!("add {}: {reason}", item.title);
                }
                Err(error) => return Err(format!("add {}: {error}", item.title)),
            }
        }

        if let Ok(mut store) = self.store.lock() {
            let mut ops = vec![
                StoreOp::Put {
                    key: meta_key("etag", &url),
                    value: fetched.etag.unwrap_or_default().into_bytes(),
                },
                StoreOp::Put {
                    key: meta_key("modified", &url),
                    value: fetched.modified.unwrap_or_default().into_bytes(),
                },
            ];
            if ops.len() == 2 {
                ops.truncate(2);
            }
            let _ = store.batch(ops);
        }
        Ok(())
    }

    async fn add_rss_item(
        &self,
        feed: &crate::config::RssFeedConfig,
        config: &crate::config::ResolvedConfig,
        item: &rss::Item,
        torrent_url: &str,
    ) -> Result<u64, EngineError> {
        let label = feed.label.trim().to_owned();
        if !label.is_empty()
            && self
                .labels
                .lock()
                .expect("label state poisoned")
                .get(&label)
                .is_none()
        {
            return Err(EngineError::Rejected {
                reason: format!("label {label:?} is not defined"),
            });
        }
        let save_path = crate::diskguard::paths::download_path(config, &feed.save_path, &label)
            .map_err(|reason| EngineError::Rejected { reason })?;

        let source = if torrent_url.starts_with("magnet:") {
            TorrentSource::Magnet(torrent_url.to_owned())
        } else {
            let target = torrent_url.to_owned();
            let bytes = tokio::task::spawn_blocking(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(20)))
                    .build()
                    .into();
                let response = agent
                    .get(&target)
                    .call()
                    .map_err(|error| error.to_string())?;
                let status = response.status().as_u16();
                if !(200..300).contains(&status) {
                    return Err(format!("HTTP {status}"));
                }
                response
                    .into_body()
                    .read_to_vec()
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| EngineError::Rejected {
                reason: error.to_string(),
            })?
            .map_err(|reason| EngineError::Rejected { reason })?;
            TorrentSource::Metainfo(bytes)
        };

        let name = if item.title.is_empty() {
            "rss item".to_owned()
        } else {
            item.title.clone()
        };
        let metainfo = match &source {
            TorrentSource::Metainfo(bytes) => bytes.clone(),
            TorrentSource::Magnet(_) => Vec::new(),
        };
        let spec = AddTorrent {
            name: name.clone(),
            total_bytes: 0,
            source,
            save_path: save_path.clone(),
            resume_data: None,
        };
        let engine = Arc::clone(&self.engine);
        let id = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            engine.add_torrent(spec)
        })
        .await
        .map_err(|error| EngineError::Rejected {
            reason: error.to_string(),
        })??;

        let persisted = PersistedTorrent {
            id,
            name,
            save_path,
            metainfo,
            resume_data: None,
            paused_by_user: false,
            label: if label.is_empty() { None } else { Some(label) },
        };
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            persist::save_torrent(&mut **store, &persisted)
        })
        .await
        .map_err(|error| EngineError::Rejected {
            reason: error.to_string(),
        })?
        .map_err(|error| EngineError::Rejected {
            reason: error.to_string(),
        })?;
        Ok(id)
    }
}

/// Starts the RSS loop; it re-reads the config every tick so edits apply.
///
/// The first pass runs immediately, so a fresh start picks up new items
/// without waiting a whole poll interval.
pub fn spawn_rss(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            let _ = state.rss_tick().await;
            let poll = state
                .config
                .resolved()
                .map(|config| config.rss.poll_secs.clamp(60, 86_400))
                .unwrap_or(300);
            tokio::time::sleep(Duration::from_secs(poll)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StateStore;

    #[test]
    fn status_keys_round_trip_through_a_store() {
        let mut store = crate::store::MemoryStore::new();
        store
            .batch(vec![StoreOp::Put {
                key: seen_key("https://f", "g-1"),
                value: b"3".to_vec(),
            }])
            .expect("write");
        assert_eq!(store.list("rss-seen:https://f:").expect("list").len(), 1);
    }

    #[test]
    fn preview_classifies_torrents_and_the_filter() {
        let item = |title: &str, link: &str, enclosure: &str| rss::Item {
            title: title.to_owned(),
            id: title.to_owned(),
            link: link.to_owned(),
            enclosure: enclosure.to_owned(),
        };
        let items = vec![
            item("Movie.2024.1080p", "", "https://x/a.torrent"),
            item("Movie.2024.720p", "", "https://x/b.torrent"),
            item("An article", "https://x/page", ""),
            item("Magnet.2024.1080p", "magnet:?xt=urn:btih:abc", ""),
        ];
        let filter = regex::Regex::new("1080p").expect("regex");
        let report = classify(items, Some(&filter));
        assert_eq!(report.total, 4);
        assert!(report.items[0].matched);
        assert_eq!(report.items[1].reason, "filter");
        assert_eq!(report.items[2].reason, "not a torrent");
        assert!(report.items[3].matched);

        // An empty filter keeps every torrent.
        let report = classify(vec![item("Anything", "", "https://x/c.torrent")], None);
        assert!(report.items[0].matched);
    }
}
