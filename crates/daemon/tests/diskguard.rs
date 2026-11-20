#![cfg(unix)]

//! DiskGuard storage model: zones from the config, resolution against real
//! directories and the storage payload.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::diskguard::moves::{MoveOutcome, RETRY_SECS};
use ferrent_daemon::diskguard::{self, Zone, ZoneError};
use ferrent_daemon::events::EventKind;
use ferrent_daemon::labels::{Label, LabelState};
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{AddTorrent, MockEngine, TorrentEngine, TorrentSource};

fn base(tag: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("ferrent-diskguard-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("base");
    directory
}

fn zone_toml(library: &Path, staging: &Path) -> String {
    format!(
        r#"
[diskguard]
min_free_bytes = 1048576
poll_secs = 30

[[diskguard.zone]]
path = {library:?}
label = "Filmek"

[[diskguard.zone]]
path = {staging:?}
staging = true
"#,
        library = library.display().to_string(),
        staging = staging.display().to_string(),
    )
}

#[test]
fn a_label_resolves_to_the_staging_folder_of_its_filesystem() {
    let base = base("resolve");
    let library = base.join("Filmek");
    let staging = base.join("_progress");
    std::fs::create_dir_all(&library).expect("library");
    std::fs::create_dir_all(&staging).expect("staging");

    let store = ConfigStore::default();
    store
        .apply_toml(&zone_toml(&library, &staging), 0)
        .expect("config");
    let resolved = store.resolved().expect("resolved");
    let zones = &resolved.diskguard.zone;

    assert_eq!(
        diskguard::resolve_staging(zones, "filmek").expect("resolve"),
        staging
    );
    assert_eq!(
        diskguard::resolve_staging(zones, "Other"),
        Err(ZoneError::UnknownLabel("Other".to_owned()))
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_staging_folder_on_another_filesystem_is_rejected() {
    let base = base("cross-device");
    let library = base.join("Filmek");
    std::fs::create_dir_all(&library).expect("library");
    // The system temp directory is often a tmpfs; only assert the error when
    // the two paths really sit on different filesystems.
    std::fs::create_dir_all("/tmp").expect("tmp");
    let same = match (
        diskguard::device_id(&library),
        diskguard::device_id(Path::new("/tmp")),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    };

    let zones = vec![
        Zone::library(library.clone(), "Filmek"),
        Zone::staging("/tmp"),
    ];
    let result = diskguard::resolve_staging(&zones, "Filmek");
    if same {
        assert_eq!(result, Ok(PathBuf::from("/tmp")));
    } else {
        assert!(
            matches!(result, Err(ZoneError::MissingStaging { .. })),
            "{result:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn the_storage_payload_reports_the_disk_and_its_zones() {
    let base = base("payload");
    let library = base.join("Filmek");
    let staging = base.join("_progress");
    std::fs::create_dir_all(&library).expect("library");
    std::fs::create_dir_all(&staging).expect("staging");

    let store = ConfigStore::default();
    store
        .apply_toml(&zone_toml(&library, &staging), 0)
        .expect("config");
    let resolved = store.resolved().expect("resolved");
    let torrents = vec![(1, staging.join("one")), (2, library.join("two"))];
    let report = diskguard::snapshot(&resolved.diskguard.zone, &torrents, 0)
        .await
        .expect("snapshot");

    assert_eq!(report.disks.len(), 1, "{report:?}");
    let disk = &report.disks[0];
    assert!(disk.free > 0, "{disk:?}");
    assert_eq!(disk.labels, vec!["Filmek".to_owned()]);
    assert_eq!(disk.torrents, 2);
    assert_eq!(disk.zones.len(), 2);
    assert!(!disk.low, "the threshold is zero");
    assert!(report.unknown.is_empty(), "{report:?}");

    let _ = std::fs::remove_dir_all(&base);
}

struct MoveFixture {
    base: PathBuf,
    staging: PathBuf,
    library: PathBuf,
    state: Arc<AppState>,
    id: u64,
}

impl MoveFixture {
    fn new(tag: &str, library: &Path, total_bytes: u64) -> Self {
        let base = base(tag);
        let staging = base.join("_progress");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::create_dir_all(library).expect("library");
        let config = ConfigStore::default();
        config
            .apply_toml(
                &format!(
                    r#"
[[diskguard.zone]]
path = {library:?}
label = "filmek"

[[diskguard.zone]]
path = {staging:?}
staging = true
"#,
                    library = library.display().to_string(),
                    staging = staging.display().to_string(),
                ),
                0,
            )
            .expect("config");
        let mut labels = LabelState::default();
        labels
            .upsert(Label {
                name: "filmek".to_owned(),
                ..Label::default()
            })
            .expect("label");
        let mut engine = MockEngine::new();
        let id = engine
            .add_torrent(AddTorrent {
                name: "film.mkv".to_owned(),
                total_bytes,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:film".to_owned()),
                save_path: staging.clone(),
                resume_data: None,
            })
            .expect("add");
        labels
            .assign(id, Some("filmek".to_owned()))
            .expect("assign");
        let (watch, _) = tokio::sync::broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(engine),
            Arc::new(StateMirror::new()),
            watch,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            labels,
            "token",
            base.clone(),
        ));
        Self {
            base,
            staging,
            library: library.to_owned(),
            state,
            id,
        }
    }

    fn save_path(&self) -> PathBuf {
        self.state
            .engine
            .lock()
            .expect("engine")
            .status(self.id)
            .expect("status")
            .save_path
    }
}

impl Drop for MoveFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[tokio::test]
async fn a_finished_torrent_moves_into_the_library_zone() {
    let holder = base("move-zone");
    let library = holder.join("Filmek");
    let fixture = MoveFixture::new("move-zone", &library, 1);
    assert_eq!(fixture.save_path(), fixture.staging);

    assert_eq!(fixture.state.try_move(fixture.id).await, MoveOutcome::Moved);
    assert_eq!(fixture.save_path(), fixture.library);
    assert_eq!(
        fixture.state.try_move(fixture.id).await,
        MoveOutcome::AlreadyThere
    );

    let _ = std::fs::remove_dir_all(&holder);
}

#[tokio::test]
async fn a_full_target_keeps_the_torrent_in_staging_and_retries() {
    // The library lives on the system temp filesystem (usually a tmpfs), the
    // staging on the build filesystem, so the move is a cross-disk copy that
    // needs room for the whole payload.
    let library = Path::new("/tmp").join(format!("ferrent-library-{}", std::process::id()));
    std::fs::create_dir_all(&library).expect("library");
    let same_device = diskguard::device_id(&library)
        == diskguard::device_id(Path::new(env!("CARGO_MANIFEST_DIR")));
    let fixture = MoveFixture::new("move-full", &library, u64::MAX / 2);

    fixture.state.handle_finished_move(fixture.id).await;
    if same_device {
        assert_eq!(fixture.save_path(), fixture.library);
        assert!(fixture.state.pending_moves().is_empty());
    } else {
        assert_eq!(fixture.save_path(), fixture.staging);
        let pending = fixture.state.pending_moves();
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].1.attempts, 1);
        assert!(pending[0].1.next_at > 0);

        // A second pass before the retry time changes nothing.
        assert_eq!(fixture.state.retry_moves(pending[0].1.next_at - 1).await, 0);
        assert_eq!(fixture.state.retry_moves(pending[0].1.next_at + 1).await, 1);
        let pending = fixture.state.pending_moves();
        assert_eq!(pending[0].1.attempts, 2, "{pending:?}");

        // Three more retries exhaust the budget and leave an Error event.
        let mut now = pending[0].1.next_at;
        for _ in 0..3 {
            now += RETRY_SECS;
            fixture.state.retry_moves(now).await;
        }
        assert!(fixture.state.pending_moves().is_empty());
        let events = ferrent_daemon::hooks::pending_events(Arc::clone(&fixture.state.store))
            .await
            .expect("pending");
        assert!(
            events.iter().any(|event| event.kind == EventKind::Error),
            "{events:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&library);
}

#[test]
fn the_add_path_of_a_label_points_at_its_staging() {
    let base = base("add-path");
    let staging = base.join("_progress");
    let library = base.join("Filmek");
    std::fs::create_dir_all(&staging).expect("staging");
    std::fs::create_dir_all(&library).expect("library");
    let store = ConfigStore::default();
    store
        .apply_toml(
            &format!(
                "download_dir = \"{}\"\n{}",
                base.join("downloads").display(),
                zone_toml(&library, &staging)
            ),
            0,
        )
        .expect("config");
    let resolved = store.resolved().expect("resolved");

    assert_eq!(
        diskguard::paths::download_path(&resolved, "", "filmek").expect("staging"),
        staging
    );
    assert_eq!(
        diskguard::paths::download_path(&resolved, "", "").expect("session"),
        base.join("downloads")
    );
    assert_eq!(
        diskguard::paths::download_path(&resolved, "custom", "filmek").expect("explicit"),
        base.join("downloads").join("custom")
    );
    assert_eq!(
        diskguard::paths::space_gate(&resolved, &staging),
        diskguard::paths::SpaceGate::Ok
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn a_label_change_waits_for_the_next_completion() {
    let holder = base("label-change");
    let library = holder.join("Filmek");
    let series = holder.join("Sorozatok");
    let fixture = MoveFixture::new("label-change", &library, 1);
    std::fs::create_dir_all(&series).expect("series");

    let mut raw = fixture.state.config.toml();
    raw.push_str(&format!(
        "\n[[diskguard.zone]]\npath = {:?}\nlabel = \"sorozatok\"\n",
        series.display().to_string()
    ));
    let revision = fixture.state.config.revision();
    fixture
        .state
        .config
        .apply_toml(&raw, revision)
        .expect("config");
    {
        let mut labels = fixture.state.labels.lock().expect("labels");
        labels
            .upsert(Label {
                name: "sorozatok".to_owned(),
                ..Label::default()
            })
            .expect("label");
        labels
            .assign(fixture.id, Some("sorozatok".to_owned()))
            .expect("assign");
    }

    // The label change alone does not touch the payload.
    assert_eq!(fixture.save_path(), fixture.staging);
    // The next completion follows the new zone.
    assert_eq!(fixture.state.try_move(fixture.id).await, MoveOutcome::Moved);
    assert_eq!(fixture.save_path(), series);

    let _ = std::fs::remove_dir_all(&holder);
}
