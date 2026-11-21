//! Zone-aware completion moves.
//!
//! A finished torrent is moved into its label's library zone. The move is a
//! rename when the staging folder and the zone sit on one filesystem; a
//! cross-filesystem move needs the whole payload plus a margin, so a full
//! target keeps the torrent seeding in staging and schedules a retry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ferrent_engine::TorrentId;

use crate::events::EventKind;

use super::{device_id, disk_stat, library_zone};

/// Seconds between two retries of a blocked move.
pub const RETRY_SECS: i64 = 300;
/// How often a blocked move is retried before it is given up.
pub const MAX_ATTEMPTS: u32 = 5;
/// How often the retry loop looks for due moves.
pub const TICK: Duration = Duration::from_secs(30);

/// A move waiting for space or for its target to appear.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PendingMove {
    pub attempts: u32,
    pub next_at: i64,
}

/// Result of one move attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveOutcome {
    Moved,
    AlreadyThere,
    /// The torrent has no label, so it stays where it is.
    NoTarget,
    /// The library folder does not exist (unmounted disk?).
    MissingTarget(PathBuf),
    /// The target filesystem cannot hold the payload.
    NoSpace {
        free: u64,
        needed: u64,
    },
    Failed(String),
}

impl crate::rpc::AppState {
    /// Final home of a torrent: its label's library zone, then the legacy
    /// `move_completed_path` field.
    #[must_use]
    pub fn move_target(&self, id: TorrentId) -> Option<PathBuf> {
        let label = {
            let labels = self.labels.lock().expect("label state poisoned");
            labels.label(id).map(str::to_owned)
        }?;
        if let Ok(config) = self.config.resolved() {
            if let Some(zone) = library_zone(&config.diskguard.zone, &label) {
                return Some(zone.path.clone());
            }
        }
        self.labels
            .lock()
            .expect("label state poisoned")
            .move_target(id)
    }

    /// Moves a finished torrent to its zone, or explains why it stayed.
    pub async fn try_move(&self, id: TorrentId) -> MoveOutcome {
        let status = {
            let engine = Arc::clone(&self.engine);
            match tokio::task::spawn_blocking(move || {
                let engine = engine.lock().expect("engine mutex poisoned");
                engine.status(id)
            })
            .await
            {
                Ok(Ok(status)) => status,
                Ok(Err(error)) => return MoveOutcome::Failed(error.to_string()),
                Err(error) => return MoveOutcome::Failed(error.to_string()),
            }
        };
        // A torrent added into the incomplete folder has no label target of
        // its own; its final home is the global download directory.
        let target = self.move_target(id).or_else(|| {
            let config = self.config.resolved().ok()?;
            let temp = config.session.temp_dir.as_deref()?.trim();
            if temp.is_empty() || status.save_path != std::path::Path::new(temp) {
                return None;
            }
            Some(config.download_dir.clone())
        });
        let Some(target) = target else {
            return MoveOutcome::NoTarget;
        };
        if status.save_path == target {
            return MoveOutcome::AlreadyThere;
        }
        // Create the last component of the zone, but only when its parent is
        // there: a missing parent means the disk is not mounted.
        if !target.is_dir() {
            let parent_ready = target.parent().is_some_and(|parent| parent.is_dir());
            if !parent_ready {
                return MoveOutcome::MissingTarget(target);
            }
            if let Err(error) = std::fs::create_dir_all(&target) {
                return MoveOutcome::Failed(error.to_string());
            }
        }
        if device_id(&status.save_path) != device_id(&target) {
            let free = disk_stat(&target).map_or(0, |stat| stat.free);
            if free < status.total_bytes {
                return MoveOutcome::NoSpace {
                    free,
                    needed: status.total_bytes,
                };
            }
        }

        let engine = Arc::clone(&self.engine);
        let destination = target.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            engine.move_storage(id, destination)
        })
        .await;
        match result {
            Ok(Ok(())) => {
                self.mirror.set_save_path(id, &target);
                let store = Arc::clone(&self.store);
                let persisted = target.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let mut store = store.lock().expect("state store mutex poisoned");
                    crate::persist::save_torrent_path(&mut **store, id, &persisted)
                })
                .await;
                MoveOutcome::Moved
            }
            Ok(Err(error)) => MoveOutcome::Failed(error.to_string()),
            Err(error) => MoveOutcome::Failed(error.to_string()),
        }
    }

    /// Handles the `Finished` event: moves, or schedules a retry and journals
    /// the reason so hooks and notifications see it.
    pub async fn handle_finished_move(&self, id: TorrentId) {
        let outcome = self.try_move(id).await;
        match outcome {
            MoveOutcome::Moved | MoveOutcome::AlreadyThere | MoveOutcome::NoTarget => {}
            MoveOutcome::MissingTarget(path) => {
                self.schedule_move(id, crate::events::now_unix());
                self.publish_move_problem(
                    id,
                    format!("move target {} is not available; retrying", path.display()),
                )
                .await;
            }
            MoveOutcome::NoSpace { free, needed } => {
                self.schedule_move(id, crate::events::now_unix());
                self.publish_move_problem(
                    id,
                    format!(
                        "not enough space for the move: {free} bytes free, {needed} needed; retrying"
                    ),
                )
                .await;
            }
            MoveOutcome::Failed(message) => {
                eprintln!("label move failed for torrent {id}: {message}");
            }
        }
    }

    /// Retries due moves; returns how many were attempted.
    pub async fn retry_moves(&self, now: i64) -> usize {
        let due: Vec<(TorrentId, PendingMove)> = {
            let mut moves = self.moves.lock().expect("move queue poisoned");
            let ids: Vec<TorrentId> = moves
                .iter()
                .filter(|(_, pending)| pending.next_at <= now)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| moves.remove(&id).map(|pending| (id, pending)))
                .collect()
        };
        let mut attempted = 0;
        for (id, pending) in due {
            attempted += 1;
            match self.try_move(id).await {
                MoveOutcome::Moved | MoveOutcome::AlreadyThere | MoveOutcome::NoTarget => {}
                outcome => {
                    let attempts = pending.attempts.saturating_add(1);
                    if attempts >= MAX_ATTEMPTS {
                        self.publish_move_problem(
                            id,
                            format!("move gave up after {attempts} attempts ({outcome:?})"),
                        )
                        .await;
                    } else {
                        self.moves.lock().expect("move queue poisoned").insert(
                            id,
                            PendingMove {
                                attempts,
                                next_at: now.saturating_add(RETRY_SECS),
                            },
                        );
                    }
                }
            }
        }
        attempted
    }

    /// Moves waiting for space or a target folder, for status surfaces.
    #[must_use]
    pub fn pending_moves(&self) -> Vec<(TorrentId, PendingMove)> {
        pending(&self.moves.lock().expect("move queue poisoned"))
    }

    fn schedule_move(&self, id: TorrentId, now: i64) {
        let mut moves = self.moves.lock().expect("move queue poisoned");
        let pending = moves.entry(id).or_default();
        pending.attempts = pending.attempts.saturating_add(1);
        pending.next_at = now.saturating_add(RETRY_SECS);
    }

    async fn publish_move_problem(&self, id: TorrentId, message: String) {
        let name = self
            .mirror
            .get(id)
            .map(|status| status.name.clone())
            .unwrap_or_default();
        let event =
            crate::events::NewEvent::new(EventKind::Error, id, name).with_error(Some(message));
        if let Err(error) = self.publish_event(event).await {
            eprintln!("hook journal: {error}");
        }
    }
}

/// Starts the move retry loop.
pub fn spawn_move_retry(state: Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            state.retry_moves(crate::events::now_unix()).await;
        }
    });
}

/// A short summary of the move queue, for status surfaces.
#[must_use]
pub fn pending(moves: &BTreeMap<TorrentId, PendingMove>) -> Vec<(TorrentId, PendingMove)> {
    moves.iter().map(|(id, pending)| (*id, *pending)).collect()
}
