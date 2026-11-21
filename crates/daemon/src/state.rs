//! In-memory state mirror. The engine's event loop is the only writer; RPC
//! readers load an immutable snapshot through `ArcSwap`, so the read path takes
//! no locks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arc_swap::ArcSwap;
use ferrent_engine::{EngineEvent, TorrentId, TorrentState, TorrentStatus};
use tokio::sync::broadcast;

/// How often the writer publishes a new snapshot to readers.
pub const PUBLISH_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TorrentSort {
    #[default]
    Id,
    Name,
    Progress,
    DownloadRate,
    UploadRate,
    Queue,
    Label,
}

/// Server-side query. `limit == 0` means "no limit".
#[derive(Debug, Clone, Default)]
pub struct TorrentQuery {
    /// Case-insensitive substring match on the torrent name.
    pub filter: Option<String>,
    /// Restrict to one lifecycle state, using the proto enum code.
    pub state: Option<i32>,
    /// Restrict to one label name.
    pub label: Option<String>,
    pub sort: TorrentSort,
    pub descending: bool,
    pub offset: usize,
    pub limit: usize,
}

/// Sort key for the download queue: torrents without an engine position last.
fn queue_key(status: &TorrentStatus) -> i32 {
    if status.queue_position < 0 {
        i32::MAX
    } else {
        status.queue_position
    }
}

/// Proto `TorrentState` code for a status, so the query and the GUI agree.
#[must_use]
pub fn state_code(state: &TorrentState) -> i32 {
    match state {
        TorrentState::Queued => 1,
        TorrentState::Checking => 2,
        TorrentState::Downloading => 3,
        TorrentState::Seeding => 4,
        TorrentState::Paused => 5,
        TorrentState::Error(_) => 6,
        TorrentState::Moving => 7,
    }
}

#[derive(Debug)]
struct MirrorData {
    torrents: BTreeMap<TorrentId, Arc<TorrentStatus>>,
    /// Download queue order; the index is the torrent's queue position.
    order: Vec<TorrentId>,
    dirty: BTreeSet<TorrentId>,
}

/// Reassigns queue positions from the order vector.
fn normalize_queue(data: &mut MirrorData) {
    for (index, id) in data.order.iter().enumerate() {
        if let Some(status) = data.torrents.get_mut(id) {
            Arc::make_mut(status).queue_position = index as i32;
        }
    }
}

/// Concurrent state mirror.
pub struct StateMirror {
    snapshot: ArcSwap<BTreeMap<TorrentId, Arc<TorrentStatus>>>,
    data: Mutex<MirrorData>,
    snapshot_dirty: AtomicBool,
}

impl Default for StateMirror {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMirror {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(BTreeMap::new()),
            data: Mutex::new(MirrorData {
                torrents: BTreeMap::new(),
                order: Vec::new(),
                dirty: BTreeSet::new(),
            }),
            snapshot_dirty: AtomicBool::new(false),
        }
    }

    /// Replaces the mirror contents and publishes immediately.
    pub fn seed(&self, statuses: impl IntoIterator<Item = TorrentStatus>) {
        {
            let mut data = self.data.lock().expect("state mirror poisoned");
            for status in statuses {
                let id = status.id;
                data.dirty.insert(id);
                data.torrents.insert(id, Arc::new(status));
                if !data.order.contains(&id) {
                    data.order.push(id);
                }
            }
            normalize_queue(&mut data);
        }
        self.snapshot_dirty.store(true, Ordering::Release);
        self.publish();
    }

    /// Moves a torrent one slot up or down in the download queue.
    ///
    /// Returns `false` when the id is unknown; a torrent already at the edge
    /// stays put but still counts as found.
    pub fn move_in_queue(&self, id: TorrentId, up: bool) -> bool {
        {
            let mut data = self.data.lock().expect("state mirror poisoned");
            let Some(index) = data.order.iter().position(|queued| *queued == id) else {
                return false;
            };
            let target = if up {
                index.checked_sub(1)
            } else if index + 1 < data.order.len() {
                Some(index + 1)
            } else {
                None
            };
            if let Some(target) = target {
                data.order.swap(index, target);
                normalize_queue(&mut data);
            } else {
                return true;
            }
        }
        self.snapshot_dirty.store(true, Ordering::Release);
        self.publish();
        true
    }

    /// Applies one engine delta. The snapshot becomes visible on `publish`.
    pub fn apply(&self, event: EngineEvent) {
        {
            let mut data = self.data.lock().expect("state mirror poisoned");
            match event {
                EngineEvent::Added(status) => {
                    let id = status.id;
                    let mut status = status;
                    let position = match data.order.iter().position(|queued| *queued == id) {
                        Some(index) => index,
                        None => {
                            data.order.push(id);
                            data.order.len() - 1
                        }
                    };
                    status.queue_position = position as i32;
                    data.dirty.insert(id);
                    data.torrents.insert(id, Arc::new(status));
                }
                EngineEvent::StateChanged { id, state } => {
                    let changed = if let Some(status) = data.torrents.get_mut(&id) {
                        Arc::make_mut(status).state = state;
                        true
                    } else {
                        false
                    };
                    if changed {
                        data.dirty.insert(id);
                    }
                }
                EngineEvent::Progress {
                    id,
                    progress,
                    download_rate,
                    upload_rate,
                    queue_position: _,
                    seeds,
                    peers,
                } => {
                    let changed = if let Some(status) = data.torrents.get_mut(&id) {
                        let status = Arc::make_mut(status);
                        status.progress = progress.clamp(0.0, 1.0);
                        status.download_rate = download_rate;
                        status.upload_rate = upload_rate;
                        // The mirror is the only source for the table's
                        // seeds/peers columns, so the counts travel with the
                        // progress event.
                        status.seeds = seeds.max(0) as u32;
                        status.peers = peers.max(0) as u32;
                        true
                    } else {
                        false
                    };
                    if changed {
                        data.dirty.insert(id);
                    }
                }
                EngineEvent::Finished { id } => {
                    if let Some(status) = data.torrents.get_mut(&id) {
                        Arc::make_mut(status).progress = 1.0;
                        data.dirty.insert(id);
                    }
                }
                EngineEvent::Removed { id } => {
                    data.torrents.remove(&id);
                    if let Some(index) = data.order.iter().position(|queued| *queued == id) {
                        data.order.remove(index);
                        normalize_queue(&mut data);
                    }
                    data.dirty.insert(id);
                }
            }
        }
        self.snapshot_dirty.store(true, Ordering::Release);
    }

    /// Publishes the current writer state to readers. A no-op when unchanged.
    pub fn publish(&self) {
        if !self.snapshot_dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let torrents = {
            let data = self.data.lock().expect("state mirror poisoned");
            data.torrents.clone()
        };
        self.snapshot.store(Arc::new(torrents));
    }

    /// Returns and clears the set of torrents changed since the last call.
    ///
    /// Removed torrents are included so a persistence layer can delete them.
    pub fn take_dirty(&self) -> BTreeSet<TorrentId> {
        let mut data = self.data.lock().expect("state mirror poisoned");
        std::mem::take(&mut data.dirty)
    }

    /// Every known torrent, in id order.
    #[must_use]
    pub fn all(&self) -> Vec<TorrentStatus> {
        self.snapshot
            .load()
            .values()
            .map(|status| (**status).clone())
            .collect()
    }

    /// Replaces the recorded save path of one torrent.
    ///
    /// The engine reports a storage move only as a state change, so the daemon
    /// records the new path itself once the move succeeded.
    pub fn set_save_path(&self, id: TorrentId, path: &std::path::Path) -> bool {
        let changed = {
            let mut data = self.data.lock().expect("state mirror poisoned");
            if let Some(status) = data.torrents.get_mut(&id) {
                Arc::make_mut(status).save_path = path.to_owned();
                data.dirty.insert(id);
                true
            } else {
                false
            }
        };
        if changed {
            self.snapshot_dirty.store(true, Ordering::Release);
        }
        changed
    }

    /// Returns a torrent snapshot.
    #[must_use]
    pub fn get(&self, id: TorrentId) -> Option<TorrentStatus> {
        self.snapshot
            .load()
            .get(&id)
            .map(|status| status.as_ref().clone())
    }

    /// Number of torrents in the published snapshot.
    #[must_use]
    pub fn len(&self) -> usize {
        self.snapshot.load().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Runs a server-side filter/sort/page query against the published snapshot.
    #[must_use]
    pub fn query(&self, query: &TorrentQuery) -> Vec<TorrentStatus> {
        self.query_page(query).0
    }

    /// Like [`Self::query`], but also returns the total match count before
    /// paging.
    #[must_use]
    pub fn query_page(&self, query: &TorrentQuery) -> (Vec<TorrentStatus>, usize) {
        self.query_page_with_labels(query, &BTreeMap::new())
    }

    /// Like [`Self::query_page`], with the daemon-owned label assignments
    /// available for filtering and sorting.
    #[must_use]
    pub fn query_page_with_labels(
        &self,
        query: &TorrentQuery,
        labels: &BTreeMap<TorrentId, String>,
    ) -> (Vec<TorrentStatus>, usize) {
        let snapshot = self.snapshot.load_full();
        let needle = query.filter.as_ref().map(|filter| filter.to_lowercase());

        let mut items: Vec<&Arc<TorrentStatus>> = snapshot
            .values()
            .filter(|status| {
                needle
                    .as_ref()
                    .is_none_or(|needle| status.name.to_lowercase().contains(needle))
                    && query
                        .state
                        .is_none_or(|state| state_code(&status.state) == state)
                    && query
                        .label
                        .as_ref()
                        .is_none_or(|label| labels.get(&status.id) == Some(label))
            })
            .collect();

        items.sort_unstable_by(|left, right| {
            let ordering = match query.sort {
                TorrentSort::Id => left.id.cmp(&right.id),
                TorrentSort::Name => left.name.cmp(&right.name),
                TorrentSort::Progress => left.progress.total_cmp(&right.progress),
                TorrentSort::DownloadRate => left.download_rate.cmp(&right.download_rate),
                TorrentSort::UploadRate => left.upload_rate.cmp(&right.upload_rate),
                TorrentSort::Queue => queue_key(left).cmp(&queue_key(right)),
                TorrentSort::Label => {
                    let left = labels.get(&left.id).map(String::as_str).unwrap_or("");
                    let right = labels.get(&right.id).map(String::as_str).unwrap_or("");
                    left.cmp(right)
                }
            };
            let ordering = if query.descending {
                ordering.reverse()
            } else {
                ordering
            };
            ordering.then_with(|| left.id.cmp(&right.id))
        });

        let total = items.len();
        let limit = if query.limit == 0 {
            usize::MAX
        } else {
            query.limit
        };
        let page = items
            .into_iter()
            .skip(query.offset)
            .take(limit)
            .map(|status| status.as_ref().clone())
            .collect();
        (page, total)
    }
}

/// Applies engine events to the mirror and publishes snapshots periodically.
///
/// The task owns the only writer path; readers never wait on the mutex.
pub async fn run_mirror(
    mirror: Arc<StateMirror>,
    mut events: broadcast::Receiver<EngineEvent>,
    watch: broadcast::Sender<EngineEvent>,
) {
    let mut ticker = tokio::time::interval(PUBLISH_INTERVAL);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    mirror.apply(event.clone());
                    let _ = watch.send(event);
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    eprintln!("state mirror lagged by {skipped} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ticker.tick() => mirror.publish(),
        }
    }

    while let Ok(event) = events.try_recv() {
        mirror.apply(event);
    }
    mirror.publish();
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use std::path::PathBuf;

    use ferrent_engine::TorrentState;

    use super::*;

    fn status(id: TorrentId, name: &str) -> TorrentStatus {
        TorrentStatus {
            id,
            infohash: format!("{id:040x}"),
            name: name.to_owned(),
            save_path: PathBuf::from("/tmp"),
            total_bytes: 1024,
            state: TorrentState::Downloading,
            progress: 0.5,
            download_rate: 0,
            upload_rate: 0,
            uploaded_bytes: 0,
            downloaded_bytes: 0,
            seeds: 0,
            peers: 0,
            eta_seconds: 0,
            queue_position: 1,
        }
    }

    fn seed_4000(mirror: &StateMirror) {
        mirror.seed((0..4_000).map(|id| TorrentStatus {
            progress: (id % 100) as f32 / 100.0,
            download_rate: id,
            upload_rate: id / 2,
            ..status(id, &format!("torrent-{id:04}"))
        }));
    }

    #[test]
    fn queue_moves_and_removals_renumber() {
        fn order(mirror: &StateMirror) -> Vec<u64> {
            mirror
                .query(&TorrentQuery {
                    sort: TorrentSort::Queue,
                    ..TorrentQuery::default()
                })
                .into_iter()
                .map(|torrent| torrent.id)
                .collect()
        }

        let mirror = StateMirror::new();
        mirror.seed(vec![
            status(1, "Alpha"),
            status(2, "beta"),
            status(3, "Gamma"),
        ]);
        assert_eq!(order(&mirror), vec![1, 2, 3]);

        assert!(mirror.move_in_queue(3, true));
        assert_eq!(order(&mirror), vec![1, 3, 2]);
        assert_eq!(order(&mirror), vec![1, 3, 2]);

        // Already at the top: found, but nothing changes.
        assert!(mirror.move_in_queue(1, true));
        assert_eq!(order(&mirror), vec![1, 3, 2]);

        assert!(!mirror.move_in_queue(42, false));

        mirror.apply(EngineEvent::Removed { id: 1 });
        mirror.publish();
        assert_eq!(order(&mirror), vec![3, 2]);
    }

    #[test]
    fn applies_deltas_and_queries() {
        let mirror = StateMirror::new();
        mirror.seed(vec![
            status(1, "Alpha"),
            status(2, "beta"),
            status(3, "Gamma"),
        ]);

        mirror.apply(EngineEvent::Progress {
            id: 2,
            progress: 0.9,
            download_rate: 100,
            upload_rate: 10,
            queue_position: 0,
            seeds: 0,
            peers: 0,
        });
        mirror.apply(EngineEvent::StateChanged {
            id: 1,
            state: TorrentState::Paused,
        });
        mirror.apply(EngineEvent::Removed { id: 3 });
        mirror.publish();

        assert_eq!(mirror.get(2).expect("torrent 2").progress, 0.9);
        assert_eq!(
            mirror.get(1).expect("torrent 1").state,
            TorrentState::Paused
        );
        assert!(mirror.get(3).is_none());

        let results = mirror.query(&TorrentQuery {
            filter: Some("alp".to_owned()),
            ..TorrentQuery::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Alpha");
    }

    #[test]
    fn sorts_and_paginates() {
        let mirror = StateMirror::new();
        seed_4000(&mirror);

        let (first_page, total) = mirror.query_page(&TorrentQuery {
            sort: TorrentSort::Progress,
            descending: true,
            limit: 10,
            ..TorrentQuery::default()
        });
        assert_eq!(total, 4_000);
        assert_eq!(first_page.len(), 10);
        for pair in first_page.windows(2) {
            assert!(pair[0].progress >= pair[1].progress);
        }

        let second_page = mirror.query(&TorrentQuery {
            sort: TorrentSort::Progress,
            descending: true,
            offset: 10,
            limit: 10,
            ..TorrentQuery::default()
        });
        assert_eq!(second_page.len(), 10);
        assert!(first_page.last().expect("page").progress >= second_page[0].progress);

        let all = mirror.query(&TorrentQuery {
            limit: 0,
            ..TorrentQuery::default()
        });
        assert_eq!(all.len(), 4_000);
    }

    #[test]
    fn dirty_tracking_includes_updates_and_removals() {
        let mirror = StateMirror::new();
        mirror.seed(vec![status(1, "one"), status(2, "two")]);
        assert_eq!(mirror.take_dirty().len(), 2);

        mirror.apply(EngineEvent::Progress {
            id: 2,
            progress: 0.7,
            download_rate: 1,
            upload_rate: 1,
            queue_position: 0,
            seeds: 0,
            peers: 0,
        });
        assert_eq!(mirror.take_dirty(), BTreeSet::from([2]));

        mirror.apply(EngineEvent::Removed { id: 1 });
        assert_eq!(mirror.take_dirty(), BTreeSet::from([1]));
        mirror.publish();
        assert!(mirror.get(1).is_none());
    }

    #[test]
    fn repeated_updates_do_not_grow_the_map() {
        let mirror = StateMirror::new();
        mirror.seed(vec![status(7, "one")]);
        for step in 0..100_000 {
            mirror.apply(EngineEvent::Progress {
                id: 7,
                progress: 0.5,
                download_rate: step,
                upload_rate: 0,
                queue_position: 0,
                seeds: 0,
                peers: 0,
            });
        }
        mirror.publish();
        assert_eq!(mirror.len(), 1);
        assert_eq!(mirror.take_dirty(), BTreeSet::from([7]));
    }

    #[test]
    fn label_filter_and_sort_use_the_daemon_assignments() {
        let mirror = StateMirror::new();
        mirror.seed(vec![status(1, "Alpha"), status(2, "beta")]);
        let labels = BTreeMap::from([(1, "zebra".to_owned()), (2, "alpha".to_owned())]);

        let (page, total) = mirror.query_page_with_labels(
            &TorrentQuery {
                label: Some("zebra".to_owned()),
                ..TorrentQuery::default()
            },
            &labels,
        );
        assert_eq!(total, 1);
        assert_eq!(page[0].id, 1);

        let page = mirror.query_page_with_labels(
            &TorrentQuery {
                sort: TorrentSort::Label,
                ..TorrentQuery::default()
            },
            &labels,
        );
        assert_eq!(
            page.0.iter().map(|status| status.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn query_p95_is_under_50ms_at_4000_torrents() {
        let mirror = StateMirror::new();
        seed_4000(&mirror);

        let query = TorrentQuery {
            filter: Some("torrent-1".to_owned()),
            state: None,
            label: None,
            sort: TorrentSort::DownloadRate,
            descending: true,
            offset: 0,
            limit: 50,
        };

        for _ in 0..10 {
            let _ = mirror.query(&query);
        }

        let mut durations: Vec<Duration> = (0..100)
            .map(|_| {
                let start = Instant::now();
                let _ = mirror.query(&query);
                start.elapsed()
            })
            .collect();
        durations.sort_unstable();
        let p95 = durations[(durations.len() * 95 / 100).min(durations.len() - 1)];
        println!("state mirror query p95: {p95:?}");
        assert!(p95 < Duration::from_millis(50), "p95 was {p95:?}");
    }

    #[test]
    fn label_query_p95_is_under_50ms_at_4000_torrents() {
        let mirror = StateMirror::new();
        seed_4000(&mirror);
        let labels: BTreeMap<TorrentId, String> = (0..4_000)
            .map(|id| {
                let label = if id % 2 == 0 { "even" } else { "odd" };
                (id, label.to_owned())
            })
            .collect();
        let query = TorrentQuery {
            label: Some("even".to_owned()),
            sort: TorrentSort::Label,
            limit: 50,
            ..TorrentQuery::default()
        };

        for _ in 0..10 {
            let _ = mirror.query_page_with_labels(&query, &labels);
        }
        let mut durations: Vec<Duration> = (0..100)
            .map(|_| {
                let start = Instant::now();
                let _ = mirror.query_page_with_labels(&query, &labels);
                start.elapsed()
            })
            .collect();
        durations.sort_unstable();
        let p95 = durations[(durations.len() * 95 / 100).min(durations.len() - 1)];
        println!("label query p95: {p95:?}");
        assert!(p95 < Duration::from_millis(50), "p95 was {p95:?}");
    }

    #[tokio::test]
    async fn mirror_task_publishes_and_drains_on_shutdown() {
        let mirror = Arc::new(StateMirror::new());
        let (sender, receiver) = broadcast::channel(16);
        let (watch, _) = broadcast::channel(16);
        let task = tokio::spawn(run_mirror(Arc::clone(&mirror), receiver, watch));

        for id in 0..3 {
            sender
                .send(EngineEvent::Added(status(id, &format!("torrent-{id}"))))
                .expect("send event");
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(mirror.len(), 3);

        drop(sender);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("mirror task timed out")
            .expect("mirror task");
        assert_eq!(mirror.len(), 3);
        assert_eq!(mirror.take_dirty().len(), 3);
    }
}
