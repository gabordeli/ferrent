//! Disk guard: thresholds with hysteresis, and pausing the writers of a full
//! filesystem.
//!
//! The guard never touches a torrent the user paused, and it only resumes what
//! it paused itself: the reason bitset in [`crate::control`] is shared with the
//! scheduler and the manual pause.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ferrent_engine::TorrentState;

use crate::control::PauseReason;
use crate::events::EventKind;

use super::{StorageReport, device_id, device_id_nearest, snapshot};

/// How often the guard looks at the wall clock.
pub const TICK: Duration = Duration::from_secs(5);

/// Threshold state of one filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Low,
}

/// A filesystem whose level changed in this pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub mount: PathBuf,
    pub level: Level,
    pub free: u64,
}

/// What one guard pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardPass {
    /// Filesystems currently below the floor.
    pub low: Vec<PathBuf>,
    /// Filesystems that crossed a threshold in this pass.
    pub transitions: Vec<Transition>,
    pub paused: usize,
    pub resumed: usize,
}

/// Evaluates the level of every reported filesystem with hysteresis.
///
/// A disk turns `low` below `min` and only returns to `ok` at
/// `min + margin`, so a disk hovering at the threshold does not flap.
#[must_use]
pub fn evaluate(
    report: &StorageReport,
    previous: &BTreeMap<PathBuf, Level>,
    min: u64,
    margin: u64,
) -> Vec<(PathBuf, Level, bool)> {
    report
        .disks
        .iter()
        .map(|disk| {
            let before = previous.get(&disk.mount).copied().unwrap_or(Level::Ok);
            let level = if min == 0 {
                Level::Ok
            } else if disk.free < min {
                Level::Low
            } else if disk.free >= min.saturating_add(margin) {
                Level::Ok
            } else {
                before
            };
            (disk.mount.clone(), level, level != before)
        })
        .collect()
}

/// States that still write to the disk (seeding does not).
#[must_use]
pub fn writes(state: &TorrentState) -> bool {
    matches!(
        state,
        TorrentState::Queued
            | TorrentState::Downloading
            | TorrentState::Checking
            | TorrentState::Moving
    )
}

impl crate::rpc::AppState {
    /// Last storage payload, kept across a failed scan.
    #[must_use]
    pub fn storage_payload(&self) -> Option<StorageReport> {
        self.disks.lock().expect("disk payload poisoned").clone()
    }

    /// Scans the zones and caches the payload as the last good one.
    pub async fn scan_storage(&self) -> Option<StorageReport> {
        let config = self.config.resolved().ok()?;
        let guard = &config.diskguard;
        let torrents: Vec<(u64, PathBuf)> = self
            .mirror
            .all()
            .into_iter()
            .map(|status| (status.id, status.save_path))
            .collect();
        let report = snapshot(&guard.zone, &torrents, guard.min_free_bytes).await?;
        *self.disks.lock().expect("disk payload poisoned") = Some(report.clone());
        Some(report)
    }

    /// One guard pass: scan, evaluate, pause or resume.
    ///
    /// `None` when the guard is disabled or the scan did not finish (the
    /// caller keeps the last payload).
    pub async fn guard_tick(&self) -> Option<GuardPass> {
        let config = self.config.resolved().ok()?;
        let guard = &config.diskguard;
        if !guard.enabled {
            return None;
        }
        let report = self.scan_storage().await?;

        let previous = self
            .disk_levels
            .lock()
            .expect("disk levels poisoned")
            .clone();
        let levels = evaluate(&report, &previous, guard.min_free_bytes, guard.margin_bytes);
        Some(self.apply_levels(&report, &levels).await)
    }

    /// Applies already-evaluated levels: notifications, pause and resume.
    pub async fn apply_levels(
        &self,
        report: &StorageReport,
        levels: &[(PathBuf, Level, bool)],
    ) -> GuardPass {
        let mut pass = GuardPass::default();
        for (mount, level, changed) in levels {
            if *level == Level::Low {
                pass.low.push(mount.clone());
            }
            if !*changed {
                continue;
            }
            let free = report
                .disks
                .iter()
                .find(|disk| disk.mount == *mount)
                .map_or(0, |disk| disk.free);
            pass.transitions.push(Transition {
                mount: mount.clone(),
                level: *level,
                free,
            });
            let kind = match level {
                Level::Low => EventKind::DiskLow,
                Level::Ok => EventKind::DiskOk,
            };
            let event = crate::events::NewEvent::new(kind, 0, mount.display().to_string());
            if let Err(error) = self.publish_event(event).await {
                eprintln!("hook journal: {error}");
            }
        }
        {
            let mut stored = self.disk_levels.lock().expect("disk levels poisoned");
            stored.clear();
            for (mount, level, _) in levels {
                stored.insert(mount.clone(), *level);
            }
        }

        let devices = |wanted: Level| -> BTreeSet<u64> {
            levels
                .iter()
                .filter(|(_, level, _)| *level == wanted)
                .filter_map(|(mount, _, _)| device_id(mount))
                .collect()
        };
        let low_devices = devices(Level::Low);
        let ok_devices = devices(Level::Ok);
        if low_devices.is_empty() && ok_devices.is_empty() {
            return pass;
        }

        let statuses = self.mirror.all();
        let user_paused: BTreeSet<u64> = self
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .user_paused()
            .collect();
        let candidates: Vec<u64> = statuses
            .iter()
            .filter(|status| writes(&status.state))
            .filter(|status| !user_paused.contains(&status.id))
            .filter(|status| {
                device_id_nearest(&status.save_path)
                    .is_some_and(|device| low_devices.contains(&device))
            })
            .map(|status| status.id)
            .collect();
        let paused = self
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .set_many(&candidates, PauseReason::DiskGuard, true);
        pass.paused = paused.len();
        if !paused.is_empty() {
            let _ = self
                .engine_call(move |engine| engine.pause_many(&paused))
                .await;
        }

        let guard_paused: Vec<u64> = {
            let control = self.control.lock().expect("pause control mutex poisoned");
            control.disk_guard_paused().collect()
        };
        let resume: Vec<u64> = guard_paused
            .into_iter()
            .filter(|id| {
                statuses
                    .iter()
                    .find(|status| status.id == *id)
                    .and_then(|status| device_id_nearest(&status.save_path))
                    .is_some_and(|device| ok_devices.contains(&device))
            })
            .collect();
        let resumed = self
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .set_many(&resume, PauseReason::DiskGuard, false);
        pass.resumed = resumed.len();
        if !resumed.is_empty() {
            let _ = self
                .engine_call(move |engine| engine.resume_many(&resumed))
                .await;
        }
        pass
    }
}

/// Starts the guard loop; the poll interval comes from the config, so a
/// reload takes effect on the next tick.
pub fn spawn_disk_guard(state: Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last = 0_i64;
        loop {
            ticker.tick().await;
            let poll = state
                .config
                .resolved()
                .map(|config| config.diskguard.poll_secs)
                .unwrap_or(30);
            let poll = i64::try_from(poll).unwrap_or(30).max(1);
            let now = crate::events::now_unix();
            if now - last < poll {
                continue;
            }
            last = now;
            state.guard_tick().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diskguard::{DiskReport, StorageReport};

    fn report(free: u64) -> StorageReport {
        StorageReport {
            disks: vec![DiskReport {
                mount: PathBuf::from("/mnt/usbhdd2"),
                device: "/dev/sdb1".to_owned(),
                fstype: "ext4".to_owned(),
                total: 1_000,
                free,
                used: 1_000 - free,
                low: false,
                zones: Vec::new(),
                labels: Vec::new(),
                torrents: 0,
            }],
            unknown: Vec::new(),
        }
    }

    #[test]
    fn thresholds_have_hysteresis() {
        let mut previous = BTreeMap::new();
        let levels = evaluate(&report(100), &previous, 200, 100);
        assert_eq!(levels[0], (PathBuf::from("/mnt/usbhdd2"), Level::Low, true));
        previous.insert(PathBuf::from("/mnt/usbhdd2"), Level::Low);

        // Above the floor but inside the margin: stays low.
        let levels = evaluate(&report(250), &previous, 200, 100);
        assert_eq!(levels[0].1, Level::Low);
        assert!(!levels[0].2);

        // At min + margin it recovers.
        let levels = evaluate(&report(300), &previous, 200, 100);
        assert_eq!(levels[0].1, Level::Ok);
        assert!(levels[0].2);

        previous.insert(PathBuf::from("/mnt/usbhdd2"), Level::Ok);
        let levels = evaluate(&report(200), &previous, 200, 100);
        assert_eq!(levels[0].1, Level::Ok, "exactly min is still ok");
        assert!(!levels[0].2);
    }

    #[test]
    fn a_zero_threshold_never_goes_low() {
        let levels = evaluate(&report(0), &BTreeMap::new(), 0, 0);
        assert_eq!(levels[0].1, Level::Ok);
        assert!(!levels[0].2);
    }

    #[test]
    fn seeding_does_not_write() {
        assert!(writes(&TorrentState::Downloading));
        assert!(writes(&TorrentState::Queued));
        assert!(writes(&TorrentState::Checking));
        assert!(writes(&TorrentState::Moving));
        assert!(!writes(&TorrentState::Seeding));
        assert!(!writes(&TorrentState::Paused));
    }
}
