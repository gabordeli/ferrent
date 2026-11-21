//! Ratio / seed-time policy against a real `AppState` (M8-T3).

use std::path::PathBuf;
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{
    AddTorrent, MockEngine, TorrentEngine, TorrentSource, TorrentState, TorrentStatus,
};

fn status(id: u64, state: TorrentState, uploaded: u64, downloaded: u64) -> TorrentStatus {
    TorrentStatus {
        id,
        infohash: format!("{id:040x}"),
        name: format!("torrent-{id}"),
        save_path: PathBuf::from("/data"),
        total_bytes: 1_000,
        state,
        progress: 1.0,
        download_rate: 0,
        upload_rate: 0,
        uploaded_bytes: uploaded,
        downloaded_bytes: downloaded,
        seeds: 0,
        peers: 0,
        eta_seconds: 0,
        queue_position: -1,
    }
}

struct Fixture {
    state: Arc<AppState>,
}

impl Fixture {
    fn new(tag: &str, ratio_toml: &str, statuses: Vec<TorrentStatus>) -> Self {
        let base = std::env::temp_dir().join(format!("ferrent-ratio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("base dir");

        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!("data_dir = \"{}\"\n{ratio_toml}", base.display()),
                0,
            )
            .expect("ratio config");

        let mirror = StateMirror::new();
        mirror.seed(statuses.clone());
        let mut engine = MockEngine::new();
        for status in &statuses {
            engine
                .add_torrent(AddTorrent {
                    name: status.name.clone(),
                    total_bytes: status.total_bytes,
                    source: TorrentSource::Magnet(format!(
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
            base,
        ));
        Self { state }
    }

    async fn tick(&self) {
        self.state.ratio_tick().await.expect("ratio tick");
    }

    fn paused_by_ratio(&self) -> Vec<u64> {
        let control = self.state.control.lock().expect("control");
        (0..8)
            .filter(|id| control.reasons(*id).bits() & 8 != 0)
            .collect()
    }

    fn engine_state(&self, id: u64) -> Option<TorrentState> {
        self.state
            .engine
            .lock()
            .expect("engine")
            .status(id)
            .ok()
            .map(|status| status.state)
    }
}

#[tokio::test]
async fn a_reached_stop_ratio_pauses_with_the_ratio_reason() {
    let fixture = Fixture::new(
        "stop",
        "[ratio]\nenabled = true\nstop_at_ratio = 1.5\npoll_secs = 60\n",
        vec![status(0, TorrentState::Seeding, 1_600, 1_000)],
    );
    fixture.tick().await;
    assert_eq!(fixture.paused_by_ratio(), vec![0]);
    assert_eq!(fixture.engine_state(0), Some(TorrentState::Paused));
}

#[tokio::test]
async fn a_below_threshold_torrent_is_left_alone() {
    let fixture = Fixture::new(
        "keep",
        "[ratio]\nenabled = true\nstop_at_ratio = 1.5\npoll_secs = 60\n",
        vec![status(0, TorrentState::Seeding, 1_200, 1_000)],
    );
    fixture.tick().await;
    assert!(fixture.paused_by_ratio().is_empty());
    assert_ne!(fixture.engine_state(0), Some(TorrentState::Paused));
}

#[tokio::test]
async fn a_reached_remove_ratio_removes_the_torrent() {
    let fixture = Fixture::new(
        "remove",
        "[ratio]\nenabled = true\nremove_at_ratio = 2.0\npoll_secs = 60\n",
        vec![status(0, TorrentState::Seeding, 2_500, 1_000)],
    );
    fixture.tick().await;
    assert!(fixture.engine_state(0).is_none(), "the torrent is gone");
}

#[tokio::test]
async fn a_dry_run_only_records_the_candidates() {
    let fixture = Fixture::new(
        "dry",
        "[ratio]\nenabled = true\nstop_at_ratio = 1.0\ndry_run = true\npoll_secs = 60\n",
        vec![status(0, TorrentState::Seeding, 1_500, 1_000)],
    );
    fixture.tick().await;
    assert!(fixture.paused_by_ratio().is_empty());
    assert_ne!(fixture.engine_state(0), Some(TorrentState::Paused));
    let status = fixture.state.ratio_status.lock().expect("status").clone();
    assert!(status.enabled && status.dry_run);
    assert_eq!(status.candidates.len(), 1);
    assert!(status.candidates[0].contains("stop"));
}

#[tokio::test]
async fn a_label_override_wins_over_the_session_default() {
    let fixture = Fixture::new(
        "label",
        "[ratio]\nenabled = true\nstop_at_ratio = 1.0\npoll_secs = 60\n\n\
         [[ratio.label]]\nname = \"music\"\nstop_at_ratio = 3.0\n",
        vec![
            status(0, TorrentState::Seeding, 1_500, 1_000),
            status(1, TorrentState::Seeding, 1_500, 1_000),
        ],
    );
    {
        let mut labels = fixture.state.labels.lock().expect("labels");
        labels
            .upsert(ferrent_daemon::labels::Label {
                name: "music".to_owned(),
                ..ferrent_daemon::labels::Label::default()
            })
            .expect("label");
        labels.assign(1, Some("music".to_owned())).expect("assign");
    }
    fixture.tick().await;
    assert_eq!(fixture.paused_by_ratio(), vec![0], "only the defaulted one");
}

#[tokio::test]
async fn an_unlabelled_downloading_torrent_is_ignored() {
    let fixture = Fixture::new(
        "downloading",
        "[ratio]\nenabled = true\nstop_at_ratio = 1.0\npoll_secs = 60\n",
        vec![status(0, TorrentState::Downloading, 5_000, 1_000)],
    );
    fixture.tick().await;
    assert!(fixture.paused_by_ratio().is_empty());
}
