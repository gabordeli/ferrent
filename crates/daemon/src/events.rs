//! Durable hook event journal.
//!
//! Every hook-relevant transition is appended before its hooks run; the record
//! is deleted only after every matching hook finished. A crash in between
//! replays the event on the next start (at-least-once delivery), so hooks must
//! be idempotent: the contract passes the same torrent metadata again.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::store::{StateStore, StoreError};

/// Event record key prefix.
pub const EVENT_PREFIX: &str = "hookevent:";
/// Next event sequence number.
pub const NEXT_KEY: &str = "hookevent:next";
/// Run record key prefix.
pub const RUN_PREFIX: &str = "hookrun:";
/// Next run sequence number.
pub const RUN_NEXT_KEY: &str = "hookrun:next";
/// How many completed runs the store keeps for the GUI.
pub const MAX_RUNS: usize = 200;

/// One hook-relevant torrent transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Added,
    Finished,
    Removed,
    Error,
}

impl EventKind {
    pub const ALL: [Self; 4] = [Self::Added, Self::Finished, Self::Removed, Self::Error];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Finished => "finished",
            Self::Removed => "removed",
            Self::Error => "error",
        }
    }

    #[must_use]
    pub fn from_config(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "added" => Some(Self::Added),
            "finished" | "complete" => Some(Self::Finished),
            "removed" => Some(Self::Removed),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The metadata handed to hooks for one transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookEvent {
    pub seq: u64,
    pub kind: EventKind,
    pub torrent_id: u64,
    pub name: String,
    pub save_path: String,
    pub infohash: String,
    /// Engine error text for [`EventKind::Error`].
    pub error: Option<String>,
    pub at_unix: i64,
}

/// An event before it receives its sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub kind: EventKind,
    pub torrent_id: u64,
    pub name: String,
    pub save_path: String,
    pub infohash: String,
    pub error: Option<String>,
}

impl NewEvent {
    #[must_use]
    pub fn new(kind: EventKind, torrent_id: u64, name: impl Into<String>) -> Self {
        Self {
            kind,
            torrent_id,
            name: name.into(),
            save_path: String::new(),
            infohash: String::new(),
            error: None,
        }
    }

    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.save_path = path.into();
        self
    }

    #[must_use]
    pub fn with_infohash(mut self, infohash: impl Into<String>) -> Self {
        self.infohash = infohash.into();
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: Option<String>) -> Self {
        self.error = error;
        self
    }
}

/// One finished hook execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRun {
    pub seq: u64,
    pub event_seq: u64,
    pub event: EventKind,
    pub torrent_id: u64,
    pub torrent_name: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub at_unix: i64,
}

impl HookRun {
    /// Whether the hook finished successfully.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

fn event_key(seq: u64) -> String {
    format!("{EVENT_PREFIX}{seq:020}")
}

fn run_key(seq: u64) -> String {
    format!("{RUN_PREFIX}{seq:020}")
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn read_u64(store: &dyn StateStore, key: &str) -> Result<u64, StoreError> {
    Ok(store
        .get(key)?
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0))
}

/// Appends one event and returns it with its sequence number.
pub fn append(store: &mut dyn StateStore, new: NewEvent) -> Result<HookEvent, StoreError> {
    let next = read_u64(store, NEXT_KEY)?.saturating_add(1);
    let event = HookEvent {
        seq: next,
        kind: new.kind,
        torrent_id: new.torrent_id,
        name: new.name,
        save_path: new.save_path,
        infohash: new.infohash,
        error: new.error,
        at_unix: now_unix(),
    };
    let bytes =
        serde_json::to_vec(&event).map_err(|error| StoreError::Backend(error.to_string()))?;
    store.batch(vec![
        crate::store::StoreOp::Put {
            key: event_key(next),
            value: bytes,
        },
        crate::store::StoreOp::Put {
            key: NEXT_KEY.to_owned(),
            value: next.to_le_bytes().to_vec(),
        },
    ])?;
    Ok(event)
}

/// Returns every unacknowledged event, oldest first.
pub fn pending(store: &dyn StateStore) -> Result<Vec<HookEvent>, StoreError> {
    let mut events: Vec<HookEvent> = store
        .list(EVENT_PREFIX)?
        .into_iter()
        .filter(|(key, _)| key.as_str() != NEXT_KEY)
        .filter_map(|(_, bytes)| serde_json::from_slice(&bytes).ok())
        .collect();
    events.sort_by_key(|event| event.seq);
    Ok(events)
}

/// Removes one event from the pending set.
pub fn ack(store: &mut dyn StateStore, seq: u64) -> Result<(), StoreError> {
    store.delete(&event_key(seq))
}

/// Number of unacknowledged events.
pub fn pending_len(store: &dyn StateStore) -> Result<usize, StoreError> {
    Ok(pending(store)?.len())
}

/// Stores one hook run and prunes the oldest records.
pub fn record_run(store: &mut dyn StateStore, mut run: HookRun) -> Result<HookRun, StoreError> {
    let next = read_u64(store, RUN_NEXT_KEY)?.saturating_add(1);
    run.seq = next;
    if run.at_unix == 0 {
        run.at_unix = now_unix();
    }
    let bytes = serde_json::to_vec(&run).map_err(|error| StoreError::Backend(error.to_string()))?;
    store.batch(vec![
        crate::store::StoreOp::Put {
            key: run_key(next),
            value: bytes,
        },
        crate::store::StoreOp::Put {
            key: RUN_NEXT_KEY.to_owned(),
            value: next.to_le_bytes().to_vec(),
        },
    ])?;

    let mut runs = run_records(store)?;
    if runs.len() > MAX_RUNS {
        runs.sort_by_key(|run| run.seq);
        let excess = runs.len() - MAX_RUNS;
        let ops: Vec<crate::store::StoreOp> = runs
            .into_iter()
            .take(excess)
            .map(|run| crate::store::StoreOp::Delete {
                key: run_key(run.seq),
            })
            .collect();
        store.batch(ops)?;
    }
    Ok(run)
}

fn run_records(store: &dyn StateStore) -> Result<Vec<HookRun>, StoreError> {
    Ok(store
        .list(RUN_PREFIX)?
        .into_iter()
        .filter(|(key, _)| key.as_str() != RUN_NEXT_KEY)
        .filter_map(|(_, bytes)| serde_json::from_slice(&bytes).ok())
        .collect())
}

/// Returns the most recent runs, newest first.
pub fn recent_runs(store: &dyn StateStore, limit: usize) -> Result<Vec<HookRun>, StoreError> {
    let mut runs = run_records(store)?;
    runs.sort_by_key(|run| std::cmp::Reverse(run.seq));
    runs.truncate(limit);
    Ok(runs)
}

/// Looks up one torrent's label at run time; the journal does not freeze it.
#[must_use]
pub fn label_for(labels: &crate::labels::LabelState, torrent_id: u64) -> String {
    labels.label(torrent_id).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn event(kind: EventKind, id: u64) -> NewEvent {
        NewEvent::new(kind, id, format!("torrent-{id}"))
            .with_path(format!("/data/{id}"))
            .with_infohash("aa".repeat(20))
    }

    #[test]
    fn appended_events_are_pending_until_acked() {
        let mut store = MemoryStore::new();
        let first = append(&mut store, event(EventKind::Added, 1)).expect("append");
        let second = append(&mut store, event(EventKind::Finished, 2)).expect("append");
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);

        let queued = pending(&store).expect("pending");
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].name, "torrent-1");
        assert_eq!(queued[1].kind, EventKind::Finished);

        ack(&mut store, first.seq).expect("ack");
        let queued = pending(&store).expect("pending");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].seq, second.seq);
    }

    #[test]
    fn the_journal_survives_a_reopen() {
        let base = std::env::temp_dir().join(format!("ferrent-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("dir");
        {
            let mut store = crate::store::RedbStore::open(base.join("state.redb")).expect("open");
            append(&mut store, event(EventKind::Added, 7)).expect("append");
            store.flush().expect("flush");
        }
        {
            let store = crate::store::RedbStore::open(base.join("state.redb")).expect("reopen");
            let queued = pending(&store).expect("pending");
            assert_eq!(queued.len(), 1, "an unacked event must replay");
            assert_eq!(queued[0].torrent_id, 7);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn runs_are_recorded_newest_first_and_pruned() {
        let mut store = MemoryStore::new();
        for index in 0..(MAX_RUNS + 5) {
            record_run(
                &mut store,
                HookRun {
                    seq: 0,
                    event_seq: 1,
                    event: EventKind::Added,
                    torrent_id: index as u64,
                    torrent_name: format!("t{index}"),
                    command: "/bin/true".to_owned(),
                    exit_code: Some(0),
                    timed_out: false,
                    duration_ms: 1,
                    stdout: String::new(),
                    stderr: String::new(),
                    at_unix: 1,
                },
            )
            .expect("record");
        }
        let runs = recent_runs(&store, 10).expect("runs");
        assert_eq!(runs.len(), 10);
        assert_eq!(runs[0].torrent_name, format!("t{}", MAX_RUNS + 4));
        assert!(runs[0].succeeded());
        assert!(runs[0].seq > runs[1].seq);

        let all = recent_runs(&store, MAX_RUNS + 100).expect("runs");
        assert_eq!(all.len(), MAX_RUNS, "pruning keeps the newest records");
    }

    #[test]
    fn event_kinds_map_to_the_config_names() {
        for (text, kind) in [
            ("added", EventKind::Added),
            ("finished", EventKind::Finished),
            ("complete", EventKind::Finished),
            ("removed", EventKind::Removed),
            ("error", EventKind::Error),
        ] {
            assert_eq!(EventKind::from_config(text), Some(kind));
        }
        assert_eq!(EventKind::from_config("nope"), None);
    }
}
