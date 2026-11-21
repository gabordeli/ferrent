//! Ratio and seed-time auto-remove.
//!
//! A torrent's limits come from its label override when that label sets them,
//! otherwise from the session defaults. Zero means "off" on both levels, so a
//! torrent is only acted on when a limit is actually configured.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ferrent_engine::TorrentId;
use serde::{Deserialize, Serialize};

use crate::config::schema::{LabelRatio, RatioConfig};

/// The effective limits for one torrent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    pub stop_at_ratio: f32,
    pub remove_at_ratio: f32,
    pub seed_time_hours: u32,
    pub remove_with_data: bool,
}

impl Policy {
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.stop_at_ratio <= 0.0 && self.remove_at_ratio <= 0.0 && self.seed_time_hours == 0
    }

    /// Label overrides win field by field; a zero field inherits the default.
    #[must_use]
    pub fn resolve(defaults: &RatioConfig, label: Option<&LabelRatio>) -> Self {
        let override_value = |value: f32, inherited: f32| {
            if value > 0.0 { value } else { inherited }
        };
        let Some(label) = label else {
            return Self {
                stop_at_ratio: defaults.stop_at_ratio,
                remove_at_ratio: defaults.remove_at_ratio,
                seed_time_hours: defaults.seed_time_hours,
                remove_with_data: defaults.remove_with_data,
            };
        };
        Self {
            stop_at_ratio: override_value(label.stop_at_ratio, defaults.stop_at_ratio),
            remove_at_ratio: override_value(label.remove_at_ratio, defaults.remove_at_ratio),
            seed_time_hours: if label.seed_time_hours > 0 {
                label.seed_time_hours
            } else {
                defaults.seed_time_hours
            },
            remove_with_data: label.remove_with_data || defaults.remove_with_data,
        }
    }
}

/// What to do with a seeding torrent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    /// Stop seeding but keep the torrent (and its data).
    Stop,
    Remove {
        delete_files: bool,
    },
}

/// `uploaded / downloaded`, with the torrent's size as the denominator when
/// less was downloaded (a torrent added with data already in place would
/// otherwise report an infinite ratio).
#[must_use]
pub fn ratio_of(uploaded: u64, downloaded: u64, total: u64) -> f32 {
    let base = downloaded.max(total).max(1);
    uploaded as f32 / base as f32
}

/// Decides what a seeding torrent deserves. `seeded_secs` is how long this
/// daemon has seen it seeding.
#[must_use]
pub fn evaluate(
    policy: &Policy,
    uploaded: u64,
    downloaded: u64,
    total: u64,
    seeded_secs: u64,
) -> Verdict {
    if policy.is_off() {
        return Verdict::Keep;
    }
    let ratio = ratio_of(uploaded, downloaded, total);
    let exhausted =
        policy.seed_time_hours > 0 && seeded_secs >= u64::from(policy.seed_time_hours) * 3_600;
    if policy.remove_at_ratio > 0.0 && ratio >= policy.remove_at_ratio {
        return Verdict::Remove {
            delete_files: policy.remove_with_data,
        };
    }
    if exhausted {
        return Verdict::Remove {
            delete_files: policy.remove_with_data,
        };
    }
    if policy.stop_at_ratio > 0.0 && ratio >= policy.stop_at_ratio {
        return Verdict::Stop;
    }
    Verdict::Keep
}

/// Runtime state of the policy loop: when each torrent started seeding, and
/// the last dry-run candidates.
#[derive(Debug, Default)]
pub struct RatioState {
    seeded_since: BTreeMap<TorrentId, Instant>,
    /// `(id, name, ratio)` of the last evaluation, for the dry run.
    pub last_candidates: Vec<(TorrentId, String, f32)>,
}

impl RatioState {
    #[must_use]
    pub fn seeded_secs(&mut self, id: TorrentId, seeding: bool, now: Instant) -> u64 {
        if !seeding {
            self.seeded_since.remove(&id);
            return 0;
        }
        let since = *self.seeded_since.entry(id).or_insert(now);
        now.saturating_duration_since(since).as_secs()
    }

    pub fn forget(&mut self, id: TorrentId) {
        self.seeded_since.remove(&id);
    }

    pub fn set_candidates(&mut self, candidates: Vec<(TorrentId, String, f32)>) {
        self.last_candidates = candidates;
    }
}

/// Status of the policy for the daemon's `GetStatus` (and the dry run).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RatioStatus {
    pub enabled: bool,
    pub dry_run: bool,
    pub candidates: Vec<String>,
}

/// A testable slice of the loop's sleep period.
#[must_use]
pub fn poll_period(config: &RatioConfig) -> Duration {
    Duration::from_secs(config.poll_secs.clamp(30, 3_600))
}

use std::sync::Arc;

use ferrent_engine::{EngineError, TorrentEngine, TorrentState};

use crate::control::PauseReason;
use crate::rpc::AppState;

impl AppState {
    fn set_ratio_status(&self, enabled: bool, dry_run: bool, candidates: Vec<String>) {
        if let Ok(mut status) = self.ratio_status.lock() {
            status.enabled = enabled;
            status.dry_run = dry_run;
            status.candidates = candidates;
        }
    }

    async fn ratio_engine<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut dyn TorrentEngine) -> Result<T, EngineError> + Send + 'static,
    {
        let engine = Arc::clone(&self.engine);
        tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            operation(&mut **engine)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    /// One evaluation of the ratio/seed-time policy.
    pub async fn ratio_tick(&self) -> Result<(), String> {
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        if !config.ratio.enabled {
            self.set_ratio_status(false, config.ratio.dry_run, Vec::new());
            return Ok(());
        }

        let now = Instant::now();
        let mut stops = Vec::new();
        let mut removals = Vec::new();
        let mut candidates = Vec::new();
        for status in self.mirror.all() {
            let seeding = status.state == TorrentState::Seeding;
            let seeded_secs = self
                .ratio
                .lock()
                .expect("ratio mutex poisoned")
                .seeded_secs(status.id, seeding, now);
            if !seeding {
                continue;
            }
            let label = self
                .labels
                .lock()
                .expect("label state poisoned")
                .label(status.id)
                .map(str::to_owned);
            let override_rule = label
                .as_deref()
                .and_then(|name| config.ratio.label.iter().find(|rule| rule.name == name));
            let policy = Policy::resolve(&config.ratio, override_rule);
            let verdict = evaluate(
                &policy,
                status.uploaded_bytes,
                status.downloaded_bytes,
                status.total_bytes,
                seeded_secs,
            );
            let ratio = ratio_of(
                status.uploaded_bytes,
                status.downloaded_bytes,
                status.total_bytes,
            );
            match verdict {
                Verdict::Keep => {}
                Verdict::Stop => {
                    candidates.push((
                        status.id,
                        format!("{} ({ratio:.2}, stop)", status.name),
                        ratio,
                    ));
                    stops.push(status.id);
                }
                Verdict::Remove { delete_files } => {
                    candidates.push((
                        status.id,
                        format!("{} ({ratio:.2}, remove)", status.name),
                        ratio,
                    ));
                    removals.push((status.id, status.name.clone(), delete_files));
                }
            }
        }

        if config.ratio.dry_run {
            let names = candidates
                .iter()
                .map(|(_, text, _)| text.clone())
                .collect::<Vec<_>>();
            self.ratio
                .lock()
                .expect("ratio mutex poisoned")
                .set_candidates(candidates);
            self.set_ratio_status(true, true, names.clone());
            for text in names {
                eprintln!("ratio dry-run: would act on {text}");
            }
            return Ok(());
        }

        let names = candidates
            .iter()
            .map(|(_, text, _)| text.clone())
            .collect::<Vec<_>>();
        self.ratio
            .lock()
            .expect("ratio mutex poisoned")
            .set_candidates(candidates);
        self.set_ratio_status(true, false, names);

        for id in stops {
            let paused = self
                .control
                .lock()
                .expect("pause control mutex poisoned")
                .set(id, PauseReason::Ratio, true);
            if paused {
                self.ratio_engine(move |engine| engine.pause(id)).await?;
            }
        }
        for (id, name, delete_files) in removals {
            self.ratio_engine(move |engine| engine.remove(id, delete_files))
                .await?;
            {
                let mut control = self.control.lock().expect("pause control mutex poisoned");
                control.clear(id);
            }
            {
                let mut ratio = self.ratio.lock().expect("ratio mutex poisoned");
                ratio.forget(id);
            }
            self.labels
                .lock()
                .expect("label state poisoned")
                .remove_torrent(id);
            {
                let mut store = self.store.lock().expect("state store poisoned");
                crate::persist::remove_torrent(&mut **store, id)
                    .map_err(|error| error.to_string())?;
            }
            eprintln!("ratio: removed {name} (#{id})");
        }
        Ok(())
    }
}

/// Starts the ratio/seed-time loop. The period comes from the config, so a
/// `SetConfig` change applies on the next wake-up.
pub fn spawn_ratio(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = state.ratio_tick().await {
                eprintln!("ratio: {error}");
            }
            let period = state
                .config
                .resolved()
                .map(|config| poll_period(&config.ratio))
                .unwrap_or_else(|_| Duration::from_secs(300));
            tokio::time::sleep(period).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(stop: f32, remove: f32, hours: u32, with_data: bool) -> Policy {
        Policy {
            stop_at_ratio: stop,
            remove_at_ratio: remove,
            seed_time_hours: hours,
            remove_with_data: with_data,
        }
    }

    #[test]
    fn zero_limits_mean_off() {
        assert!(policy(0.0, 0.0, 0, false).is_off());
        assert_eq!(
            evaluate(&policy(0.0, 0.0, 0, false), 10, 1, 100, 999_999),
            Verdict::Keep
        );
    }

    #[test]
    fn stops_at_the_stop_ratio() {
        let policy = policy(1.5, 0.0, 0, false);
        assert_eq!(evaluate(&policy, 149, 100, 0, 0), Verdict::Keep);
        assert_eq!(evaluate(&policy, 150, 100, 0, 0), Verdict::Stop);
    }

    #[test]
    fn removal_wins_over_stopping() {
        let policy = policy(1.0, 2.0, 0, true);
        assert_eq!(evaluate(&policy, 100, 100, 0, 0), Verdict::Stop);
        assert_eq!(
            evaluate(&policy, 200, 100, 0, 0),
            Verdict::Remove { delete_files: true }
        );
    }

    #[test]
    fn seed_time_expiry_removes() {
        let policy = policy(0.0, 0.0, 2, false);
        assert_eq!(evaluate(&policy, 0, 100, 100, 7_199), Verdict::Keep);
        assert_eq!(
            evaluate(&policy, 0, 100, 100, 7_200),
            Verdict::Remove {
                delete_files: false
            }
        );
    }

    #[test]
    fn ratio_uses_the_torrent_size_when_nothing_was_downloaded() {
        // A torrent added with data already present: 500 MiB uploaded of a
        // 1 GiB torrent is a ratio of 0.49, not infinity.
        assert!((ratio_of(500, 0, 1_000) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn label_overrides_fields_one_by_one() {
        let defaults = RatioConfig {
            stop_at_ratio: 1.0,
            remove_at_ratio: 2.0,
            seed_time_hours: 24,
            remove_with_data: false,
            ..RatioConfig::default()
        };
        let label = LabelRatio {
            name: "music".to_owned(),
            stop_at_ratio: 3.0,
            ..LabelRatio::default()
        };
        let resolved = Policy::resolve(&defaults, Some(&label));
        assert_eq!(resolved.stop_at_ratio, 3.0);
        assert_eq!(resolved.remove_at_ratio, 2.0, "inherited");
        assert_eq!(resolved.seed_time_hours, 24, "inherited");
    }

    #[test]
    fn seeding_time_resets_when_the_torrent_leaves_seeding() {
        let mut state = RatioState::default();
        let start = Instant::now();
        assert_eq!(state.seeded_secs(7, true, start), 0);
        assert_eq!(
            state.seeded_secs(7, true, start + Duration::from_secs(60)),
            60
        );
        assert_eq!(
            state.seeded_secs(7, false, start + Duration::from_secs(90)),
            0
        );
        assert_eq!(
            state.seeded_secs(7, true, start + Duration::from_secs(90)),
            0,
            "a new seeding window starts from zero"
        );
    }
}
