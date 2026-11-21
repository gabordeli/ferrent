//! Client-side activity log.
//!
//! The daemon keeps no event history, so the GUI derives one from what it
//! already sees: the states on every page it polls, plus its own actions. The
//! log is a bounded ring buffer, so a long session cannot grow memory.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use ferrent_protocol::v1::Torrent;

/// How many events the log keeps.
pub const CAPACITY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A change the client asked for (pause/resume/remove from this GUI).
    Client,
    /// A change the client merely observed between two polls.
    Daemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    State(i32),
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub at: SystemTime,
    pub torrent_id: u64,
    pub name: String,
    pub from: Option<i32>,
    pub change: Change,
    pub source: Source,
}

#[derive(Debug, Default)]
pub struct ActivityLog {
    known: BTreeMap<u64, i32>,
    events: VecDeque<Event>,
}

impl ActivityLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compares a polled page against the last state seen for each torrent.
    ///
    /// Paging means a page only ever holds a slice, so membership is not
    /// compared: a row that scrolls out of the page is not "removed". Only
    /// state transitions of torrents present in both polls become events.
    pub fn observe(&mut self, torrents: &[Torrent], at: SystemTime) {
        for torrent in torrents {
            match self.known.insert(torrent.id, torrent.state) {
                Some(previous) if previous != torrent.state => self.push(Event {
                    at,
                    torrent_id: torrent.id,
                    name: torrent.name.clone(),
                    from: Some(previous),
                    change: Change::State(torrent.state),
                    source: Source::Daemon,
                }),
                _ => {}
            }
        }
    }

    /// Records a pause/resume the client itself issued, before the optimistic
    /// update hides the transition from the next poll.
    pub fn client_states(&mut self, torrents: &BTreeMap<u64, Torrent>, state: i32, at: SystemTime) {
        for torrent in torrents.values() {
            self.known.insert(torrent.id, state);
            if torrent.state == state {
                continue;
            }
            self.push(Event {
                at,
                torrent_id: torrent.id,
                name: torrent.name.clone(),
                from: Some(torrent.state),
                change: Change::State(state),
                source: Source::Client,
            });
        }
    }

    /// Records a removal the client issued; the torrent is gone from the next
    /// poll, so nothing else would report it.
    pub fn client_removed(&mut self, torrents: &BTreeMap<u64, Torrent>, at: SystemTime) {
        for torrent in torrents.values() {
            let from = self.known.remove(&torrent.id);
            self.push(Event {
                at,
                torrent_id: torrent.id,
                name: torrent.name.clone(),
                from,
                change: Change::Removed,
                source: Source::Client,
            });
        }
    }

    /// Newest first, optionally only for one torrent.
    pub fn iter(&self, only: Option<u64>) -> impl Iterator<Item = &Event> {
        self.events
            .iter()
            .rev()
            .filter(move |event| only.is_none_or(|id| event.torrent_id == id))
    }

    #[must_use]
    pub fn is_empty(&self, only: Option<u64>) -> bool {
        self.iter(only).next().is_none()
    }

    /// Size of the ring buffer, capped at [`CAPACITY`].
    #[must_use]
    pub fn stored(&self) -> usize {
        self.events.len()
    }

    fn push(&mut self, event: Event) {
        if self.events.len() == CAPACITY {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn torrent(id: u64, name: &str, state: i32) -> Torrent {
        Torrent {
            id,
            name: name.to_owned(),
            state,
            ..Torrent::default()
        }
    }

    fn page(entries: &[(u64, &str, i32)]) -> BTreeMap<u64, Torrent> {
        entries
            .iter()
            .map(|(id, name, state)| (*id, torrent(*id, name, *state)))
            .collect()
    }

    #[test]
    fn records_a_state_transition_once() {
        let mut log = ActivityLog::new();
        log.observe(&[torrent(1, "a.bin", 1)], at(10));
        log.observe(&[torrent(1, "a.bin", 1)], at(20));
        assert!(log.is_empty(None), "an unchanged state is not an event");

        log.observe(&[torrent(1, "a.bin", 2)], at(30));
        let events: Vec<&Event> = log.iter(None).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, Some(1));
        assert_eq!(events[0].change, Change::State(2));
        assert_eq!(events[0].source, Source::Daemon);
        assert_eq!(events[0].at, at(30));
    }

    #[test]
    fn client_actions_are_attributed_to_the_client() {
        let mut log = ActivityLog::new();
        log.observe(&[torrent(7, "a.bin", 1), torrent(8, "b.bin", 1)], at(10));
        let selected = page(&[(7, "a.bin", 1), (8, "b.bin", 1)]);

        log.client_states(&selected, 4, at(20));
        let events: Vec<&Event> = log.iter(None).collect();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.source == Source::Client));

        // The next poll agrees with the optimistic state, so it adds nothing.
        log.observe(&[torrent(7, "a.bin", 4), torrent(8, "b.bin", 4)], at(30));
        assert_eq!(log.iter(None).count(), 2);
    }

    #[test]
    fn removal_keeps_the_previous_state() {
        let mut log = ActivityLog::new();
        log.observe(&[torrent(3, "c.bin", 3)], at(10));
        log.client_removed(&page(&[(3, "c.bin", 3)]), at(20));
        let event = log.iter(None).next().expect("event");
        assert_eq!(event.from, Some(3));
        assert_eq!(event.change, Change::Removed);
    }

    #[test]
    fn filters_by_torrent() {
        let mut log = ActivityLog::new();
        log.observe(&[torrent(1, "a.bin", 1), torrent(2, "b.bin", 1)], at(10));
        log.observe(&[torrent(1, "a.bin", 2), torrent(2, "b.bin", 3)], at(20));
        assert_eq!(log.iter(None).count(), 2);
        assert_eq!(log.iter(Some(2)).count(), 1);
        assert!(log.is_empty(Some(9)));
    }

    #[test]
    fn keeps_only_the_last_events() {
        let mut log = ActivityLog::new();
        for step in 0..CAPACITY as u64 + 20 {
            let state = if step % 2 == 0 { 1 } else { 2 };
            log.observe(&[torrent(1, "a.bin", state)], at(step));
        }
        assert_eq!(log.stored(), CAPACITY);
        let newest = log.iter(None).next().expect("event");
        assert_eq!(newest.at, at(CAPACITY as u64 + 19));
    }
}
