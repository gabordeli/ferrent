//! Interface binding and link watching (M8-T4).
//!
//! The session is bound to a listen/outgoing interface by the engine; this
//! module watches the interface a VPN tunnel lives on, so a lost tunnel pauses
//! the torrents instead of leaking traffic outside it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrent_engine::{EngineError, TorrentEngine, TorrentState};

use crate::control::PauseReason;
use crate::rpc::AppState;

/// How often the link is sampled.
const POLL: Duration = Duration::from_secs(5);

/// Whether the interface exists in the kernel.
#[must_use]
pub fn link_present(name: &str) -> bool {
    !name.is_empty() && Path::new("/sys/class/net").join(name).exists()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAction {
    /// The interface disappeared: pause everything.
    Pause,
    /// It has been back for the grace period: let the torrents run again.
    Resume,
}

/// Debounces the link: a down link is acted on at once, an up link only after
/// it stayed up for the grace period.
#[derive(Debug, Default)]
pub struct LinkWatch {
    down: bool,
    up_since: Option<Instant>,
}

impl LinkWatch {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one sample in; returns what to do, if anything.
    pub fn poll(&mut self, present: bool, now: Instant, grace: Duration) -> Option<LinkAction> {
        if !present {
            self.up_since = None;
            if self.down {
                return None;
            }
            self.down = true;
            return Some(LinkAction::Pause);
        }
        if !self.down {
            return None;
        }
        let since = *self.up_since.get_or_insert(now);
        if now.saturating_duration_since(since) < grace {
            return None;
        }
        self.down = false;
        self.up_since = None;
        Some(LinkAction::Resume)
    }
}

impl AppState {
    async fn link_engine<T, F>(&self, operation: F) -> Result<T, String>
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

    /// Applies one link action to every torrent.
    pub async fn link_action(&self, action: LinkAction) -> Result<(), String> {
        for status in self.mirror.all() {
            let id = status.id;
            let (paused, changed) = {
                let mut control = self.control.lock().expect("pause control mutex poisoned");
                let changed = control.set(id, PauseReason::Network, action == LinkAction::Pause);
                (control.is_paused(id), changed)
            };
            if !changed {
                continue;
            }
            if paused {
                self.link_engine(move |engine| engine.pause(id)).await?;
            } else if status.state != TorrentState::Paused {
                self.link_engine(move |engine| engine.resume(id)).await?;
            }
        }
        Ok(())
    }
}

/// Watches `watch_interface` and pauses on loss.
pub fn spawn_netbind(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut watch = LinkWatch::new();
        loop {
            let (interface, grace) = state
                .config
                .resolved()
                .map(|config| {
                    (
                        config.network.watch_interface.clone(),
                        Duration::from_secs(config.network.grace_secs),
                    )
                })
                .unwrap_or_default();
            if !interface.is_empty() {
                if let Some(action) = watch.poll(link_present(&interface), Instant::now(), grace) {
                    eprintln!("netbind: {interface} -> {action:?}");
                    if let Err(error) = state.link_action(action).await {
                        eprintln!("netbind: {error}");
                    }
                }
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_link_pauses_once() {
        let mut watch = LinkWatch::new();
        let now = Instant::now();
        assert_eq!(
            watch.poll(false, now, Duration::from_secs(60)),
            Some(LinkAction::Pause)
        );
        assert_eq!(watch.poll(false, now, Duration::from_secs(60)), None);
    }

    #[test]
    fn a_returning_link_waits_for_the_grace_period() {
        let mut watch = LinkWatch::new();
        let start = Instant::now();
        assert_eq!(
            watch.poll(false, start, Duration::from_secs(60)),
            Some(LinkAction::Pause)
        );
        assert_eq!(watch.poll(true, start, Duration::from_secs(60)), None);
        assert_eq!(
            watch.poll(
                true,
                start + Duration::from_secs(30),
                Duration::from_secs(60)
            ),
            None,
            "still inside the grace period"
        );
        assert_eq!(
            watch.poll(
                true,
                start + Duration::from_secs(61),
                Duration::from_secs(60)
            ),
            Some(LinkAction::Resume)
        );
    }

    #[test]
    fn a_link_that_drops_again_restarts_the_grace_period() {
        let mut watch = LinkWatch::new();
        let start = Instant::now();
        watch.poll(false, start, Duration::from_secs(60));
        watch.poll(true, start, Duration::from_secs(60));
        assert_eq!(
            watch.poll(
                false,
                start + Duration::from_secs(10),
                Duration::from_secs(60)
            ),
            None,
            "already paused, nothing new to do"
        );
        watch.poll(
            true,
            start + Duration::from_secs(20),
            Duration::from_secs(60),
        );
        assert_eq!(
            watch.poll(
                true,
                start + Duration::from_secs(50),
                Duration::from_secs(60)
            ),
            None
        );
        assert_eq!(
            watch.poll(
                true,
                start + Duration::from_secs(81),
                Duration::from_secs(60)
            ),
            Some(LinkAction::Resume)
        );
    }

    #[test]
    fn the_loopback_exists_and_a_bogus_name_does_not() {
        assert!(link_present("lo"));
        assert!(!link_present("no-such-interface0"));
    }
}
