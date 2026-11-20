#![cfg(unix)]

//! AutoAdd service tests: real watch directories, `MockEngine` and the memory
//! store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrent_daemon::autoadd::{self, OnSuccess, WatchDir};
use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::{Label, LabelState};
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::{MemoryStore, RedbStore, StateStore};
use ferrent_engine::MockEngine;
use ferrent_engine::TorrentState;

struct Fixture {
    base: PathBuf,
    watch: PathBuf,
    state: Arc<AppState>,
}

impl Fixture {
    fn new(tag: &str, watch: WatchDir) -> Self {
        let base =
            std::env::temp_dir().join(format!("ferrent-autoadd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let watch_dir = base.join("watch");
        std::fs::create_dir_all(&watch_dir).expect("watch dir");

        let config = ConfigStore::default();
        let mut watch = watch;
        if watch.path.as_os_str().is_empty() {
            watch.path = watch_dir.clone();
        }
        let mut toml = format!(
            "[[autoadd]]\npath = {:?}\nenabled = {}\non_success = \"{}\"\n",
            watch.path.display().to_string(),
            watch.enabled,
            match watch.on_success {
                OnSuccess::Delete => "delete",
                OnSuccess::Keep => "keep",
                OnSuccess::Move => "move",
            }
        );
        if let Some(move_to) = &watch.move_to {
            toml.push_str(&format!("move_to = {:?}\n", move_to.display().to_string()));
        }
        if let Some(label) = &watch.label {
            toml.push_str(&format!("label = {label:?}\n"));
        }
        if watch.add_paused {
            toml.push_str("add_paused = true\n");
        }
        if watch.min_free_bytes > 0 {
            toml.push_str(&format!("min_free_bytes = {}\n", watch.min_free_bytes));
        }
        config.apply_toml(&toml, 0).expect("apply config");

        let (send, _) = tokio::sync::broadcast::channel(8);
        let labels = match watch.label.clone() {
            Some(name) => {
                let mut labels = LabelState::default();
                labels
                    .upsert(Label {
                        name,
                        color: "#112233".to_owned(),
                        ..Label::default()
                    })
                    .expect("label");
                labels
            }
            None => LabelState::default(),
        };
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            send,
            Box::new(MemoryStore::new()),
            config,
            TorrentControl::default(),
            labels,
            "token",
            base.clone(),
        ));
        Self {
            base,
            watch: watch_dir,
            state,
        }
    }

    fn write_torrent(&self, file_name: &str, name: &str, length: u64) {
        std::fs::write(self.watch.join(file_name), torrent_bytes(name, length)).expect("write");
    }

    fn engine_count(&self) -> usize {
        self.state.engine.lock().expect("engine").list().len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn torrent_bytes(name: &str, length: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"d4:infod6:lengthi");
    bytes.extend_from_slice(length.to_string().as_bytes());
    bytes.extend_from_slice(b"e4:name");
    bytes.extend_from_slice(name.len().to_string().as_bytes());
    bytes.push(b':');
    bytes.extend_from_slice(name.as_bytes());
    bytes.extend_from_slice(b"ee");
    bytes
}

fn watch() -> WatchDir {
    WatchDir {
        enabled: true,
        ..WatchDir::default()
    }
}

#[tokio::test]
async fn adds_once_and_deletes_duplicates() {
    let fixture = Fixture::new("once", watch());
    fixture.write_torrent("a.torrent", "movie.mkv", 10);

    // First pass only records the fingerprint.
    let first = fixture.state.scan_once().await;
    assert_eq!(first.waiting, 1);
    assert_eq!(first.added, 0);
    assert!(fixture.watch.join("a.torrent").exists());

    let second = fixture.state.scan_once().await;
    assert_eq!(second.added, 1);
    assert_eq!(fixture.engine_count(), 1);
    assert!(!fixture.watch.join("a.torrent").exists(), "deleted");

    // The same metainfo under a different file name is a duplicate: the
    // engine keeps one torrent and the file still gets its success action.
    fixture.write_torrent("b.torrent", "movie.mkv", 10);
    fixture.state.scan_once().await;
    let totals = fixture.state.scan_once().await;
    assert_eq!(totals.added, 0);
    assert_eq!(totals.duplicates, 1);
    assert_eq!(fixture.engine_count(), 1);
    assert!(!fixture.watch.join("b.torrent").exists());
}

#[tokio::test]
async fn a_hundred_files_are_picked_up_within_the_target() {
    let fixture = Fixture::new("bulk", watch());
    for index in 0..100 {
        fixture.write_torrent(
            &format!("{index:03}.torrent"),
            &format!("bulk-{index}.mkv"),
            10 + index,
        );
    }
    let warm = fixture.state.scan_once().await;
    assert_eq!(warm.waiting, 100);

    let started = std::time::Instant::now();
    let totals = fixture.state.scan_once().await;
    let elapsed = started.elapsed();
    assert_eq!(totals.added, 100, "{totals:?}");
    assert_eq!(fixture.engine_count(), 100);
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "100 adds took {elapsed:?}"
    );
}

#[tokio::test]
async fn invalid_files_become_dot_invalid_after_three_attempts() {
    let fixture = Fixture::new("invalid", watch());
    std::fs::write(fixture.watch.join("broken.torrent"), b"not bencode").expect("write");

    // The first pass records the fingerprint, then three attempts follow.
    fixture.state.scan_once().await;
    for _ in 0..2 {
        fixture.state.scan_once().await;
    }
    assert!(fixture.watch.join("broken.torrent").exists());
    assert_eq!(fixture.engine_count(), 0);

    let totals = fixture.state.scan_once().await;
    assert_eq!(totals.invalid, 1, "{totals:?}");
    assert!(!fixture.watch.join("broken.torrent").exists());
    assert!(
        fixture.watch.join("broken.torrent.invalid").exists(),
        "marked .invalid"
    );

    // A zero-byte file takes the same path.
    std::fs::write(fixture.watch.join("empty.torrent"), b"").expect("write");
    for _ in 0..4 {
        fixture.state.scan_once().await;
    }
    assert!(
        fixture.watch.join("empty.torrent.invalid").exists(),
        "a zero-byte file is marked .invalid"
    );
}

#[tokio::test]
async fn the_free_space_gate_keeps_files_in_the_watch_directory() {
    let fixture = Fixture::new(
        "gate",
        WatchDir {
            min_free_bytes: 1_u64 << 62,
            enabled: true,
            ..WatchDir::default()
        },
    );
    fixture.write_torrent("waiting.torrent", "waiting.mkv", 10);

    let first = fixture.state.scan_once().await;
    assert_eq!(first.waiting, 1);
    let totals = fixture.state.scan_once().await;
    assert_eq!(totals.deferred, 1, "{totals:?}");
    assert_eq!(fixture.engine_count(), 0);
    assert!(fixture.watch.join("waiting.torrent").exists());

    // The gate is not an attempt: several scans must not mark it invalid.
    for _ in 0..4 {
        fixture.state.scan_once().await;
    }
    assert!(fixture.watch.join("waiting.torrent").exists());
    let status = fixture.state.autoadd_status();
    assert!(status[0].1.deferred >= 1, "{status:?}");
    assert!(!fixture.watch.join("waiting.torrent.invalid").exists());
}

#[tokio::test]
async fn magnet_lines_add_paused_torrents_with_the_label() {
    let fixture = Fixture::new(
        "magnet",
        WatchDir {
            label: Some("movies".to_owned()),
            add_paused: true,
            enabled: true,
            ..WatchDir::default()
        },
    );
    std::fs::write(
        fixture.watch.join("links.magnet"),
        "magnet:?xt=urn:btih:aaa&dn=First\nmagnet:?xt=urn:btih:bbb&dn=Second\n",
    )
    .expect("write");

    fixture.state.scan_once().await;
    let totals = fixture.state.scan_once().await;
    let status = fixture.state.autoadd_status();
    assert_eq!(totals.added, 2, "{totals:?} {status:?}");
    assert!(!fixture.watch.join("links.magnet").exists());

    let statuses = fixture.state.engine.lock().expect("engine").list();
    assert_eq!(statuses.len(), 2);
    for status in &statuses {
        assert_eq!(status.state, TorrentState::Paused, "add_paused");
        assert_eq!(
            fixture
                .state
                .labels
                .lock()
                .expect("labels")
                .label(status.id),
            Some("movies")
        );
    }
    let names: Vec<&str> = statuses.iter().map(|status| status.name.as_str()).collect();
    assert!(names.contains(&"First"), "{names:?}");
    assert!(names.contains(&"Second"), "{names:?}");
}

#[tokio::test]
async fn move_action_puts_the_file_in_the_destination() {
    let base = std::env::temp_dir().join(format!("ferrent-autoadd-move-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let done = base.join("done");
    let fixture = Fixture::new(
        "move",
        WatchDir {
            on_success: OnSuccess::Move,
            move_to: Some(done.clone()),
            enabled: true,
            ..WatchDir::default()
        },
    );
    fixture.write_torrent("moved.torrent", "moved.mkv", 10);
    fixture.state.scan_once().await;
    fixture.state.scan_once().await;

    assert_eq!(fixture.engine_count(), 1);
    assert!(!fixture.watch.join("moved.torrent").exists());
    assert!(done.join("moved.torrent").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn attempt_counters_survive_a_store_reopen() {
    let base = std::env::temp_dir().join(format!("ferrent-autoadd-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("dir");
    let path = Path::new("/data/watch/slow.torrent");

    {
        let mut store = RedbStore::open(base.join("state.redb")).expect("open");
        assert_eq!(autoadd::record_failure(&mut store, path).expect("count"), 1);
        assert_eq!(autoadd::record_failure(&mut store, path).expect("count"), 2);
        store.flush().expect("flush");
    }
    {
        let store = RedbStore::open(base.join("state.redb")).expect("reopen");
        assert_eq!(autoadd::attempts(&store, path), 2);
        let _ = store.get("nothing");
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn a_low_staging_disk_defers_the_add() {
    let fixture = Fixture::new("lowdisk", watch());
    let raw = fixture.state.config.toml();
    let revision = fixture.state.config.revision();
    fixture
        .state
        .config
        .apply_toml(
            &format!("{raw}\n[diskguard]\nmin_free_bytes = {}\n", u64::MAX / 2),
            revision,
        )
        .expect("config");
    fixture.write_torrent("waiting.torrent", "waiting.mkv", 10);

    fixture.state.scan_once().await;
    let totals = fixture.state.scan_once().await;
    assert_eq!(totals.deferred, 1, "{totals:?}");
    assert_eq!(fixture.engine_count(), 0);
    assert!(
        fixture.watch.join("waiting.torrent").exists(),
        "the file stays for a later scan"
    );
    assert!(!fixture.watch.join("waiting.torrent.invalid").exists());
}

#[tokio::test]
async fn a_preallocate_gate_defers_the_add() {
    let fixture = Fixture::new("prealloc", watch());
    let raw = fixture.state.config.toml();
    let revision = fixture.state.config.revision();
    fixture
        .state
        .config
        .apply_toml(
            &format!(
                "{raw}\n[diskguard]\nrequire_space_for_torrent = true\nmargin_bytes = {}\n",
                u64::MAX / 2
            ),
            revision,
        )
        .expect("config");
    fixture.write_torrent("big.torrent", "big.mkv", 10);

    fixture.state.scan_once().await;
    let totals = fixture.state.scan_once().await;
    assert_eq!(totals.deferred, 1, "{totals:?}");
    assert_eq!(fixture.engine_count(), 0);
    assert!(
        fixture.watch.join("big.torrent").exists(),
        "stays for later"
    );
    assert!(!fixture.watch.join("big.torrent.invalid").exists());
}
