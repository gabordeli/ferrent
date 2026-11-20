#![cfg(unix)]

//! Execute hook tests: real processes, a temporary watch file, journal replay.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::events::{self, EventKind, NewEvent};
use ferrent_daemon::hooks;
use ferrent_daemon::labels::{Label, LabelState};
use ferrent_daemon::rpc::AppState;
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{AddTorrent, MockEngine, TorrentSource};

struct Fixture {
    base: PathBuf,
    state: Arc<AppState>,
}

impl Fixture {
    fn new(tag: &str, config: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ferrent-hooks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("base dir");
        let store = ConfigStore::default();
        store.apply_toml(config, 0).expect("apply config");
        let (send, _) = tokio::sync::broadcast::channel(8);
        let state = Arc::new(AppState::new(
            Box::new(MockEngine::new()),
            Arc::new(StateMirror::new()),
            send,
            Box::new(MemoryStore::new()),
            store,
            TorrentControl::default(),
            LabelState::default(),
            "token",
            base.clone(),
        ));
        Self { base, state }
    }

    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.base.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod");
        path
    }

    fn apply(&self, config: &str) {
        let revision = self.state.config.revision();
        self.state
            .config
            .apply_toml(config, revision)
            .expect("apply config");
    }

    async fn append(&self, event: NewEvent) {
        self.state.append_hook_event(event).await.expect("append");
    }

    async fn pending(&self) -> Vec<ferrent_daemon::events::HookEvent> {
        hooks::pending_events(Arc::clone(&self.state.store))
            .await
            .expect("pending")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn hook_config(events: &[(&str, u64)]) -> String {
    events
        .iter()
        .map(|(kind, timeout)| {
            format!(
                "[[hook]]\nevent = \"{kind}\"\ncommand = \"/bin/true\"\ntimeout_secs = {timeout}\n"
            )
        })
        .collect()
}

#[tokio::test]
async fn hooks_receive_argv_and_env_and_are_acknowledged() {
    let fixture = Fixture::new("argv", "");
    let output = fixture.base.join("out.txt");
    let script = fixture.script(
        "capture.sh",
        &format!(
            "{{ printf 'argv=%s\\n' \"$*\"; printf 'env=%s|%s|%s|%s\\n' \"$FERRENT_EVENT\" \"$FERRENT_ID\" \"$FERRENT_NAME\" \"$FERRENT_LABEL\"; }} > {}",
            output.display()
        ),
    );
    let config = format!(
        "[[hook]]\nevent = \"added\"\ncommand = {:?}\n",
        script.display().to_string()
    );
    fixture.apply(&config);
    fixture
        .state
        .labels
        .lock()
        .expect("labels")
        .upsert(Label {
            name: "movies".to_owned(),
            ..Label::default()
        })
        .expect("label");
    {
        let mut labels = fixture.state.labels.lock().expect("labels");
        labels.assign(7, Some("movies".to_owned())).expect("assign");
    }

    fixture
        .append(
            NewEvent::new(EventKind::Added, 7, "two words.mkv")
                .with_path("/data/movies")
                .with_infohash("cd".repeat(20)),
        )
        .await;
    let executed = fixture.state.run_pending_hooks().await.expect("run hooks");
    assert_eq!(executed, 1);
    assert!(fixture.pending().await.is_empty(), "the event was acked");

    let captured = std::fs::read_to_string(&output).expect("captured output");
    assert!(
        captured.contains("argv=7 two words.mkv /data/movies"),
        "{captured}"
    );
    assert!(captured.contains(&"cd".repeat(20)), "{captured}");
    assert!(
        captured.contains("env=added|7|two words.mkv|movies"),
        "{captured}"
    );

    let runs =
        events::recent_runs(&**fixture.state.store.lock().expect("store"), 10).expect("runs");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].succeeded());
    assert_eq!(runs[0].torrent_name, "two words.mkv");
}

#[tokio::test]
async fn a_timed_out_hook_is_killed_and_recorded() {
    let fixture = Fixture::new("timeout", &hook_config(&[("added", 1)]));
    let config = "[[hook]]\nevent = \"added\"\ncommand = \"/bin/sh\"\nargs = [\"-c\", \"sleep 30\"]\ntimeout_secs = 1\n";
    fixture.apply(config);

    fixture
        .append(NewEvent::new(EventKind::Added, 1, "slow"))
        .await;
    let started = std::time::Instant::now();
    let executed = fixture.state.run_pending_hooks().await.expect("run hooks");
    let elapsed = started.elapsed();
    assert_eq!(executed, 1);
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "timeout must kill the hook, took {elapsed:?}"
    );

    let runs =
        events::recent_runs(&**fixture.state.store.lock().expect("store"), 10).expect("runs");
    assert!(runs[0].timed_out, "{:?}", runs[0]);
    assert!(!runs[0].succeeded());
    assert!(fixture.pending().await.is_empty());
}

#[tokio::test]
async fn at_most_four_hooks_run_at_once() {
    let fixture = Fixture::new("parallel", "");
    let script = fixture.script("slow.sh", "sleep 0.3");
    let mut config = String::new();
    for _ in 0..6 {
        config.push_str(&format!(
            "[[hook]]\nevent = \"added\"\ncommand = {:?}\n",
            script.display().to_string()
        ));
    }
    fixture.apply(&config);

    fixture
        .append(NewEvent::new(EventKind::Added, 1, "parallel"))
        .await;
    let started = std::time::Instant::now();
    let executed = fixture.state.run_pending_hooks().await.expect("run hooks");
    let elapsed = started.elapsed();
    assert_eq!(executed, 6);
    // Six sequential runs would take ~1.8 s; four at a time takes ~0.6 s.
    assert!(
        elapsed < std::time::Duration::from_millis(1400),
        "hooks did not run in parallel: {elapsed:?}"
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(550),
        "hooks finished suspiciously fast: {elapsed:?}"
    );
}

#[tokio::test]
async fn events_without_a_matching_hook_are_acked() {
    let fixture = Fixture::new("nomatch", &hook_config(&[("finished", 60)]));
    fixture
        .append(NewEvent::new(EventKind::Added, 3, "unmatched"))
        .await;
    assert_eq!(fixture.pending().await.len(), 1);
    let executed = fixture.state.run_pending_hooks().await.expect("run hooks");
    assert_eq!(executed, 0);
    assert!(fixture.pending().await.is_empty());
}

#[tokio::test]
async fn a_removed_event_keeps_the_pre_remove_snapshot() {
    let fixture = Fixture::new("removed", "");

    // Seed the engine and the mirror, then journal the removal the same way the
    // RPC handler does: from the mirror before the engine drops the torrent.
    let (id, status) = {
        let mut engine = fixture.state.engine.lock().expect("engine");
        let id = engine
            .add_torrent(AddTorrent {
                name: "gone.mkv".to_owned(),
                total_bytes: 1,
                source: TorrentSource::Magnet("magnet:?xt=urn:btih:gone".to_owned()),
                save_path: Path::new("/data/gone").to_path_buf(),
                resume_data: None,
            })
            .expect("add");
        (id, engine.status(id).expect("status"))
    };

    fixture
        .append(
            NewEvent::new(EventKind::Removed, id, status.name.clone())
                .with_path(status.save_path.display().to_string())
                .with_infohash(status.infohash.clone()),
        )
        .await;

    let pending = fixture.pending().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, EventKind::Removed);
    assert_eq!(pending[0].name, "gone.mkv");
    assert_eq!(pending[0].save_path, "/data/gone");
    assert!(!pending[0].infohash.is_empty());
}
