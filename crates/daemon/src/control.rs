//! Per-torrent pause reasons.
//!
//! A torrent is paused when *any* reason is active. This lets the scheduler
//! and the disk guard pause without clobbering a manual pause, and keeps the
//! engine's state as the derived value.

use std::collections::BTreeMap;

use ferrent_engine::TorrentId;

/// One source of a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// The user pressed pause.
    User,
    /// The scheduler's red band.
    Scheduler,
    /// The disk guard's low-space threshold.
    DiskGuard,
    /// A ratio/time policy.
    Ratio,
}

impl PauseReason {
    const fn bit(self) -> u8 {
        match self {
            Self::User => 1,
            Self::Scheduler => 2,
            Self::DiskGuard => 4,
            Self::Ratio => 8,
        }
    }
}

/// Bitset of active pause reasons for one torrent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseReasons(u8);

impl PauseReasons {
    pub const USER: Self = Self(1);
    pub const SCHEDULER: Self = Self(2);
    pub const DISK_GUARD: Self = Self(4);
    pub const RATIO: Self = Self(8);

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0 as u32
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn insert(&mut self, reason: PauseReason) {
        self.0 |= reason.bit();
    }

    fn remove(&mut self, reason: PauseReason) {
        self.0 &= !reason.bit();
    }
}

/// Owns the reason bitsets and derives the effective pause state.
#[derive(Debug, Default)]
pub struct TorrentControl {
    reasons: BTreeMap<TorrentId, PauseReasons>,
}

impl TorrentControl {
    #[must_use]
    pub fn reasons(&self, id: TorrentId) -> PauseReasons {
        self.reasons.get(&id).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn is_paused(&self, id: TorrentId) -> bool {
        !self.reasons(id).is_empty()
    }

    /// Sets or clears one reason. Returns whether the effective state changed.
    pub fn set(&mut self, id: TorrentId, reason: PauseReason, active: bool) -> bool {
        let before = self.is_paused(id);
        {
            let reasons = self.reasons.entry(id).or_default();
            if active {
                reasons.insert(reason);
            } else {
                reasons.remove(reason);
            }
        }
        if self
            .reasons
            .get(&id)
            .is_some_and(|reasons| reasons.is_empty())
        {
            self.reasons.remove(&id);
        }
        before != self.is_paused(id)
    }

    /// Sets or clears the manual pause reason.
    pub fn set_user(&mut self, id: TorrentId, paused: bool) -> bool {
        self.set(id, PauseReason::User, paused)
    }

    /// Applies one reason to many torrents and returns the ids whose effective
    /// state changed.
    pub fn set_many(
        &mut self,
        ids: &[TorrentId],
        reason: PauseReason,
        active: bool,
    ) -> Vec<TorrentId> {
        ids.iter()
            .copied()
            .filter(|id| self.set(*id, reason, active))
            .collect()
    }

    /// Drops every reason for a removed torrent.
    pub fn clear(&mut self, id: TorrentId) -> bool {
        self.reasons
            .remove(&id)
            .is_some_and(|reasons| !reasons.is_empty())
    }

    /// Snapshot for read-only proto mapping.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<TorrentId, PauseReasons> {
        self.reasons.clone()
    }

    /// Ids that carry the manual pause reason.
    pub fn user_paused(&self) -> impl Iterator<Item = TorrentId> + '_ {
        self.reasons
            .iter()
            .filter(|(_, reasons)| reasons.contains(PauseReasons::USER))
            .map(|(id, _)| *id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn user_pause_survives_guard_ok() {
        let mut control = TorrentControl::default();
        assert!(control.set_user(7, true));
        assert!(control.is_paused(7));
        assert_eq!(control.reasons(7).bits(), PauseReasons::USER.bits());

        assert!(!control.set(7, PauseReason::DiskGuard, true));
        assert!(control.is_paused(7));
        assert!(!control.set(7, PauseReason::DiskGuard, false));
        assert!(
            control.is_paused(7),
            "guard resume must not clear the manual pause"
        );

        assert!(control.set_user(7, false));
        assert!(!control.is_paused(7));
        assert!(control.snapshot().is_empty());
    }

    #[test]
    fn guard_pause_resumes_when_no_other_reason_remains() {
        let mut control = TorrentControl::default();
        assert!(control.set(3, PauseReason::DiskGuard, true));
        assert!(control.is_paused(3));
        assert!(!control.set_user(3, true));
        assert!(!control.set(3, PauseReason::DiskGuard, false));
        assert!(control.is_paused(3));
        assert!(control.set_user(3, false));
        assert!(!control.is_paused(3));
    }

    #[test]
    fn clear_drops_every_reason() {
        let mut control = TorrentControl::default();
        control.set_user(1, true);
        control.set(1, PauseReason::Ratio, true);
        assert!(control.clear(1));
        assert!(!control.is_paused(1));
        assert!(!control.clear(1));
    }

    #[test]
    fn bulk_ten_thousand_torrents_is_linear() {
        let mut control = TorrentControl::default();
        let ids: Vec<TorrentId> = (0..10_000).collect();

        let started = Instant::now();
        let changed = control.set_many(&ids, PauseReason::User, true);
        assert_eq!(changed.len(), 10_000);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(control.snapshot().len(), 10_000);

        let started = Instant::now();
        let changed = control.set_many(&ids, PauseReason::User, false);
        assert_eq!(changed.len(), 10_000);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(control.snapshot().is_empty());
    }
}
