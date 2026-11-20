#![cfg(unix)]

//! Scheduler apply tests against the mock engine.

use std::path::PathBuf;
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::scheduler::{Grid, Level};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{MockEngine, TorrentEngine, TorrentState, TorrentStatus};

fn status(id: u64, state: TorrentState) -> TorrentStatus {
    TorrentStatus {
        id,
        infohash: format!("{id:040x}"),
        name: format!("torrent-{id}"),
        save_path: PathBuf::from("/data"),
        total_bytes: 1,
        state,
        progress: 0.5,
        download_rate: 0,
        upload_rate: 0,
        seeds: 0,
        peers: 0,
        eta_seconds: 0,
        queue_position: -1,
    }
}

struct Fixture {
    base: PathBuf,
    state: Arc<AppState>,
}

impl Fixture {
    fn new(tag: &str, statuses: Vec<TorrentStatus>, pause_seeding: bool, week: &Grid) -> Self {
        let base =
            std::env::temp_dir().join(format!("ferrent-scheduler-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("base dir");

        let week_line = week.render().join("\", \"");
        let toml = format!(
            "[scheduler]\nenabled = true\ntimezone = \"UTC\"\ndownload_rate_limit = 1024\nupload_rate_limit = 512\npause_seeding = {pause_seeding}\nweek = [\"{week_line}\"]\n"
        );
        let config = ConfigStore::default();
        config.apply_toml(&toml, 0).expect("scheduler config");

        let mirror = StateMirror::new();
        mirror.seed(statuses.clone());
        let mut engine = MockEngine::new();
        for status in &statuses {
            engine
                .add_torrent(ferrent_engine::AddTorrent {
                    name: status.name.clone(),
                    total_bytes: status.total_bytes,
                    source: ferrent_engine::TorrentSource::Magnet(format!(
                        "magnet:?xt=urn:btih:{}",
                        status.infohash
                    )),
                    save_path: status.save_path.clone(),
                    resume_data: None,
                })
                .expect("seed engine");
        }
        let (send, _) = tokio::sync::broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(engine),
            Arc::new(mirror),
            send,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            LabelState::default(),
            "token",
            base.clone(),
        ));
        Self { base, state }
    }

    async fn apply(&self, level: Level) {
        let config = self.state.config.resolved().expect("resolved");
        self.state
            .apply_scheduler_level(level, &config)
            .await
            .expect("apply level");
    }

    fn engine_state(&self, id: u64) -> TorrentState {
        self.state
            .engine
            .lock()
            .expect("engine")
            .status(id)
            .expect("status")
            .state
    }

    fn scheduler_paused(&self) -> Vec<u64> {
        self.state
            .control
            .lock()
            .expect("control")
            .scheduler_paused()
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[tokio::test]
async fn red_pauses_running_and_green_resumes_only_its_own() {
    let fixture = Fixture::new(
        "cycle",
        vec![
            status(0, TorrentState::Downloading),
            status(1, TorrentState::Downloading),
            status(2, TorrentState::Seeding),
            status(3, TorrentState::Queued),
        ],
        true,
        &Grid::default(),
    );
    fixture
        .state
        .control
        .lock()
        .expect("control")
        .set_user(0, true);
    fixture
        .state
        .engine
        .lock()
        .expect("engine")
        .pause(0)
        .expect("pause the user torrent");

    fixture.apply(Level::Red).await;
    assert_eq!(fixture.engine_state(1), TorrentState::Paused);
    assert_eq!(fixture.engine_state(2), TorrentState::Paused);
    assert_eq!(fixture.engine_state(3), TorrentState::Paused);
    assert_eq!(fixture.engine_state(0), TorrentState::Paused);
    let paused = fixture.scheduler_paused();
    assert_eq!(
        paused,
        vec![1, 2, 3],
        "the manual pause is not scheduler-owned"
    );

    fixture.apply(Level::Green).await;
    assert_ne!(fixture.engine_state(1), TorrentState::Paused);
    assert_ne!(fixture.engine_state(2), TorrentState::Paused);
    assert_eq!(
        fixture.engine_state(0),
        TorrentState::Paused,
        "manual pause kept"
    );
    assert!(fixture.scheduler_paused().is_empty());
}

#[tokio::test]
async fn red_keeps_seeding_when_configured_to() {
    let fixture = Fixture::new(
        "seeding",
        vec![
            status(0, TorrentState::Downloading),
            status(1, TorrentState::Seeding),
        ],
        false,
        &Grid::default(),
    );
    fixture.apply(Level::Red).await;
    assert_eq!(fixture.engine_state(0), TorrentState::Paused);
    assert_ne!(fixture.engine_state(1), TorrentState::Paused);
    assert_eq!(fixture.scheduler_paused(), vec![0]);
}

#[tokio::test]
async fn yellow_applies_the_scheduler_limits() {
    let fixture = Fixture::new(
        "limits",
        vec![status(0, TorrentState::Downloading)],
        true,
        &Grid::default(),
    );
    fixture.apply(Level::Yellow).await;
    let settings = fixture.state.engine.lock().expect("engine").settings();
    assert_eq!(settings.download_rate_limit, 1024);
    assert_eq!(settings.upload_rate_limit, 512);
    fixture.apply(Level::Green).await;
    let settings = fixture.state.engine.lock().expect("engine").settings();
    assert_eq!(settings.download_rate_limit, 0, "session default restored");
}

#[tokio::test]
async fn ten_thousand_torrents_switch_well_under_a_second() {
    let statuses: Vec<TorrentStatus> = (0..10_000)
        .map(|id| status(id, TorrentState::Downloading))
        .collect();
    let fixture = Fixture::new("bulk", statuses, true, &Grid::default());
    let started = std::time::Instant::now();
    fixture.apply(Level::Red).await;
    let elapsed = started.elapsed();
    assert_eq!(fixture.scheduler_paused().len(), 10_000);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "10k pause took {elapsed:?}"
    );
    let started = std::time::Instant::now();
    fixture.apply(Level::Green).await;
    assert!(fixture.scheduler_paused().is_empty());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "10k resume took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn tick_applies_once_per_level_change_and_recomputes_after_a_jump() {
    use chrono::{Datelike, TimeZone, Timelike, Utc};

    // The grid marks "now" red and the next hour green, so a tick applies red
    // once, then green after the boundary — the same shape as a suspend jump.
    let now = Utc.with_ymd_and_hms(2026, 11, 18, 12, 30, 0).unwrap();
    let mut week = Grid::default();
    let day = usize::try_from(now.weekday().num_days_from_monday()).expect("weekday");
    let hour = usize::try_from(now.hour()).expect("hour");
    week.set(day, hour, Level::Red);
    week.set(day, (hour + 1) % 24, Level::Green);
    let fixture = Fixture::new(
        "tick",
        vec![status(0, TorrentState::Downloading)],
        true,
        &week,
    );

    assert_eq!(
        fixture.state.scheduler_tick(now).await.expect("tick"),
        Some(Level::Red)
    );
    assert_eq!(
        fixture.state.scheduler_tick(now).await.expect("tick"),
        None,
        "the same slot is not re-applied"
    );
    let next = now + chrono::Duration::hours(1);
    assert_eq!(
        fixture.state.scheduler_tick(next).await.expect("tick"),
        Some(Level::Green)
    );
    assert_eq!(fixture.state.scheduler_status().level, Some(Level::Green));
    assert!(fixture.scheduler_paused().is_empty());
}
