#![cfg(unix)]

//! Automation acceptance (M5-T6): watch directory -> add -> label -> limit ->
//! move -> finished hook -> webhook/Telegram, plus restart safety and a live
//! config reload.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::events::{EventKind, NewEvent};
use ferrent_daemon::hooks;
use ferrent_daemon::labels::{self, Label, LabelState};
use ferrent_daemon::persist::{self, PersistedTorrent};
use ferrent_daemon::rpc::{AppState, spawn_engine_watcher, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::{MemoryStore, RedbStore, StateStore};
use ferrent_engine::{
    AddTorrent, EngineError, EngineEvent, EngineSettings, MockEngine, QueueDirection,
    TorrentEngine, TorrentFileInfo, TorrentId, TorrentLimits, TorrentPeerInfo, TorrentStatus,
    TorrentTrackerInfo,
};
use tokio::sync::broadcast;

/// Shares one [`MockEngine`] between the daemon and the test, so the test can
/// drive progress and read the limits the engine actually applied.
struct SharedEngine(Arc<Mutex<MockEngine>>);

impl SharedEngine {
    fn lock(&self) -> MutexGuard<'_, MockEngine> {
        self.0.lock().expect("engine mutex poisoned")
    }
}

impl TorrentEngine for SharedEngine {
    fn add_torrent(&mut self, spec: AddTorrent) -> Result<TorrentId, EngineError> {
        self.lock().add_torrent(spec)
    }

    fn restore_torrent(
        &mut self,
        id: TorrentId,
        spec: AddTorrent,
        paused: bool,
    ) -> Result<(), EngineError> {
        self.lock().restore_torrent(id, spec, paused)
    }

    fn request_resume_data(&mut self, id: TorrentId) -> Result<(), EngineError> {
        self.lock().request_resume_data(id)
    }

    fn take_resume_data(&mut self, id: TorrentId) -> Result<Option<Vec<u8>>, EngineError> {
        self.lock().take_resume_data(id)
    }

    fn pause(&mut self, id: TorrentId) -> Result<(), EngineError> {
        self.lock().pause(id)
    }

    fn resume(&mut self, id: TorrentId) -> Result<(), EngineError> {
        self.lock().resume(id)
    }

    fn set_torrent_limits(
        &mut self,
        id: TorrentId,
        limits: TorrentLimits,
    ) -> Result<(), EngineError> {
        self.lock().set_torrent_limits(id, limits)
    }

    fn move_storage(&mut self, id: TorrentId, path: PathBuf) -> Result<(), EngineError> {
        self.lock().move_storage(id, path)
    }

    fn remove(&mut self, id: TorrentId, delete_files: bool) -> Result<(), EngineError> {
        self.lock().remove(id, delete_files)
    }

    fn move_in_queue(
        &mut self,
        id: TorrentId,
        direction: QueueDirection,
    ) -> Result<(), EngineError> {
        self.lock().move_in_queue(id, direction)
    }

    fn list(&self) -> Vec<TorrentStatus> {
        self.lock().list()
    }

    fn status(&self, id: TorrentId) -> Result<TorrentStatus, EngineError> {
        self.lock().status(id)
    }

    fn files(&mut self, id: TorrentId) -> Result<Vec<TorrentFileInfo>, EngineError> {
        self.lock().files(id)
    }

    fn peers(&mut self, id: TorrentId) -> Result<Vec<TorrentPeerInfo>, EngineError> {
        self.lock().peers(id)
    }

    fn trackers(&mut self, id: TorrentId) -> Result<Vec<TorrentTrackerInfo>, EngineError> {
        self.lock().trackers(id)
    }

    fn settings(&self) -> EngineSettings {
        self.lock().settings()
    }

    fn set_settings(&mut self, settings: EngineSettings) {
        self.lock().set_settings(settings);
    }

    fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.lock().subscribe()
    }
}

#[derive(Clone)]
struct Paths {
    base: PathBuf,
    watch: PathBuf,
    moved: PathBuf,
    downloads: PathBuf,
}

fn paths(tag: &str) -> Paths {
    let base = std::env::temp_dir().join(format!("ferrent-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let watch = base.join("watch");
    let moved = base.join("moved");
    let downloads = base.join("downloads");
    for directory in [&watch, &moved, &downloads] {
        std::fs::create_dir_all(directory).expect("directory");
    }
    Paths {
        base,
        watch,
        moved,
        downloads,
    }
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("script");
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(path, permissions).expect("chmod");
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

struct Harness {
    paths: Paths,
    state: Arc<AppState>,
    mirror: Arc<StateMirror>,
    mock: Arc<Mutex<MockEngine>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    cleanup: bool,
}

impl Harness {
    async fn assemble(
        paths: Paths,
        config: ConfigStore,
        store: Box<dyn StateStore>,
        labels: LabelState,
        restore: &[PersistedTorrent],
    ) -> Self {
        let mock = Arc::new(Mutex::new(MockEngine::new()));
        let events = mock.lock().expect("engine").subscribe();
        for (index, torrent) in restore.iter().enumerate() {
            let id = torrent.id;
            mock.lock()
                .expect("engine")
                .restore_torrent(id, torrent.clone().into_add_torrent(), true)
                .expect("restore");
            if index % 128 == 127 {
                // Keep the mirror task draining the mock's 256-event channel.
                tokio::task::yield_now().await;
            }
        }
        let mirror = Arc::new(StateMirror::new());
        let (watch, _) = broadcast::channel(2048);
        let mut tasks = vec![spawn_mirror(Arc::clone(&mirror), events, watch.clone())];
        let state = Arc::new(AppState::new(
            Box::new(SharedEngine(Arc::clone(&mock))),
            Arc::clone(&mirror),
            watch,
            store,
            config,
            TorrentControl::default(),
            labels,
            "token",
            paths.base.clone(),
        ));
        tasks.push(spawn_engine_watcher(Arc::clone(&state)));
        Self {
            paths,
            state,
            mirror,
            mock,
            tasks,
            cleanup: true,
        }
    }

    fn engine_ids(&self) -> Vec<TorrentId> {
        self.mock
            .lock()
            .expect("engine")
            .list()
            .into_iter()
            .map(|status| status.id)
            .collect()
    }

    fn write_torrent(&self, directory: &Path, file_name: &str, name: &str, length: u64) {
        std::fs::write(directory.join(file_name), torrent_bytes(name, length)).expect("write");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.paths.base);
        }
    }
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buffer);
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if buffer.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Serves exactly one request with `200` and returns the captured request.
fn fake_server() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_request(&mut stream);
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(response.as_bytes()).expect("write");
        request
    });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn the_automation_chain_runs_from_the_watch_dir_to_the_channels() {
    let paths = paths("chain");
    let (webhook_base, webhook_server) = fake_server();
    let (telegram_base, telegram_server) = fake_server();
    let added_log = paths.base.join("added.log");
    let finished_log = paths.base.join("finished.log");
    let added_hook = paths.base.join("added.sh");
    let finished_hook = paths.base.join("finished.sh");
    script(
        &added_hook,
        &format!("printf '%s\\n' \"$*\" >> {}", added_log.display()),
    );
    script(
        &finished_hook,
        &format!("printf '%s\\n' \"$*\" >> {}", finished_log.display()),
    );

    let config = ConfigStore::default();
    config
        .apply_toml(
            &format!(
                r#"
[notify]
enabled = true
digest_events = 0

[[notify.webhook]]
url = "{webhook_base}/hook"
secret = "s3cret"
events = ["finished"]

[[notify.telegram]]
bot_token = "123:abc"
chat_id = "-100"
api_base = "{telegram_base}"
events = ["finished"]

[[autoadd]]
path = {watch:?}
label = "movies"

[[diskguard.zone]]
path = {library:?}
label = "movies"

[[diskguard.zone]]
path = {staging:?}
staging = true

[[hook]]
event = "added"
command = {added_hook:?}

[[hook]]
event = "finished"
command = {finished_hook:?}
"#,
                watch = paths.watch.display().to_string(),
                library = paths.moved.display().to_string(),
                staging = paths.downloads.display().to_string(),
                added_hook = added_hook.display().to_string(),
                finished_hook = finished_hook.display().to_string(),
            ),
            0,
        )
        .expect("config");

    let mut label_state = LabelState::default();
    label_state
        .upsert(Label {
            name: "movies".to_owned(),
            color: "#112233".to_owned(),
            download_rate_limit: 1024,
            upload_rate_limit: 2048,
            ..Label::default()
        })
        .expect("label");

    let harness = Harness::assemble(
        paths.clone(),
        config,
        Box::new(MemoryStore::new()),
        label_state,
        &[],
    )
    .await;
    harness.write_torrent(&paths.watch, "movie.torrent", "movie.mkv", 4096);

    let warm = harness.state.scan_once().await;
    assert_eq!(warm.waiting, 1, "{warm:?}");
    let scanned = harness.state.scan_once().await;
    assert_eq!(scanned.added, 1, "{scanned:?}");
    assert!(
        !paths.watch.join("movie.torrent").exists(),
        "the watch file is delivered"
    );

    let ids = harness.engine_ids();
    assert_eq!(ids.len(), 1, "{ids:?}");
    let id = ids[0];
    {
        let labels = harness.state.labels.lock().expect("labels");
        assert_eq!(labels.label(id), Some("movies"));
        assert_eq!(labels.limits(id).download_rate_limit, 1024);
    }
    let limits = harness.mock.lock().expect("engine").limits(id);
    assert_eq!(
        limits.map(|limits| limits.upload_rate_limit),
        Some(2048),
        "the label limit reaches the engine"
    );

    assert_eq!(harness.state.run_pending_hooks().await.expect("hooks"), 1);
    let added = std::fs::read_to_string(&added_log).expect("added log");
    assert!(added.contains("movie.mkv"), "{added}");
    assert!(added.contains("movies"), "{added}");

    wait_until("the mirror to publish the new torrent", || {
        harness.mirror.get(id).is_some()
    })
    .await;
    harness
        .mock
        .lock()
        .expect("engine")
        .advance(id, 1.0, 0, 0)
        .expect("finish");
    wait_until("the label move to land in the mirror", || {
        harness
            .mirror
            .get(id)
            .is_some_and(|status| status.save_path == paths.moved)
    })
    .await;

    assert_eq!(harness.state.run_pending_hooks().await.expect("hooks"), 1);
    let finished = std::fs::read_to_string(&finished_log).expect("finished log");
    assert!(finished.contains("movie.mkv"), "{finished}");

    let queued = harness
        .state
        .notification_status()
        .await
        .expect("notification status");
    assert_eq!(queued.pending, 2, "{queued:?}");
    let report = harness
        .state
        .notify_tick(2_000_000_000)
        .await
        .expect("notify tick");
    assert_eq!(report.sent, 2, "{report:?}");
    assert_eq!(
        harness
            .state
            .notification_status()
            .await
            .expect("notification status")
            .pending,
        0
    );

    let webhook = webhook_server.join().expect("webhook server");
    assert!(webhook.contains("\"name\":\"movie.mkv\""), "{webhook}");
    let lower = webhook.to_ascii_lowercase();
    assert!(lower.contains("x-ferrent-signature: sha256="), "{webhook}");
    let telegram = telegram_server.join().expect("telegram server");
    assert!(
        telegram.starts_with("POST /bot123:abc/sendMessage"),
        "{telegram}"
    );
    assert!(telegram.contains("\"chat_id\":\"-100\""), "{telegram}");
}

#[test]
fn a_restart_keeps_adds_hooks_and_notifications_exactly_once() {
    let paths = paths("restart");
    let (webhook_base, webhook_server) = fake_server();
    let store_path = paths.base.join("state.redb");
    let config_toml = format!(
        r#"
[notify]
enabled = true
digest_events = 0

[[notify.webhook]]
url = "{webhook_base}/hook"
events = ["finished"]

[[autoadd]]
path = {watch:?}
label = "movies"
on_success = "keep"

[[hook]]
event = "finished"
command = "/bin/true"
"#,
        watch = paths.watch.display().to_string(),
    );

    // Two runtimes: dropping the first shuts the daemon tasks down, which is
    // what releases the redb file lock and models a real restart.
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let config = ConfigStore::default();
        config.apply_toml(&config_toml, 0).expect("config");
        let mut store = RedbStore::open(&store_path).expect("open");
        let label = Label {
            name: "movies".to_owned(),
            ..Label::default()
        };
        let mut label_state = LabelState::default();
        label_state.upsert(label.clone()).expect("label");
        labels::save_label(&mut store, &label).expect("save label");

        let mut harness =
            Harness::assemble(paths.clone(), config, Box::new(store), label_state, &[]).await;
        harness.write_torrent(&paths.watch, "kept.torrent", "kept.mkv", 1024);
        harness.state.scan_once().await;
        let scanned = harness.state.scan_once().await;
        assert_eq!(scanned.added, 1, "{scanned:?}");
        let id = harness.engine_ids()[0];

        harness
            .state
            .publish_event(
                NewEvent::new(EventKind::Finished, id, "kept.mkv")
                    .with_path(paths.downloads.display().to_string()),
            )
            .await
            .expect("publish");
        harness
            .state
            .store
            .lock()
            .expect("store")
            .flush()
            .expect("flush");
        harness.cleanup = false;
    });
    drop(runtime);

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let store = RedbStore::open(&store_path).expect("reopen");
        let persisted = persist::load_torrents(&store).expect("persisted");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].label.as_deref(), Some("movies"));
        let label_state = LabelState::load(&store, &persisted).expect("labels");
        let config = ConfigStore::default();
        config.apply_toml(&config_toml, 0).expect("config");
        let harness = Harness::assemble(
            paths.clone(),
            config,
            Box::new(store),
            label_state,
            &persisted,
        )
        .await;

        let pending = hooks::pending_events(Arc::clone(&harness.state.store))
            .await
            .expect("pending");
        // Both the add and the completion were journaled before the restart; only
        // the completion has a matching hook.
        assert_eq!(pending.len(), 2, "the journal survives the restart");
        assert!(pending.iter().any(|event| event.kind == EventKind::Added));
        assert!(
            pending
                .iter()
                .any(|event| event.kind == EventKind::Finished)
        );
        assert_eq!(harness.state.run_pending_hooks().await.expect("hooks"), 1);
        assert_eq!(
            harness.state.run_pending_hooks().await.expect("hooks"),
            0,
            "an acked event never runs twice"
        );

        let queued = harness
            .state
            .notification_status()
            .await
            .expect("notification status");
        assert_eq!(queued.pending, 1, "the outbox survives the restart");
        let report = harness
            .state
            .notify_tick(2_000_000_000)
            .await
            .expect("notify tick");
        assert_eq!(report.sent, 1, "{report:?}");
        let report = harness
            .state
            .notify_tick(2_000_000_000)
            .await
            .expect("notify tick");
        assert_eq!(report.sent, 0, "a delivered notification is not resent");

        // The watch file is still there (on_success = "keep"); the restored engine
        // recognises the infohash, so the rescan is a duplicate, not a second add.
        harness.state.scan_once().await;
        let rescan = harness.state.scan_once().await;
        assert_eq!(rescan.duplicates, 1, "{rescan:?}");
        assert_eq!(rescan.added, 0, "{rescan:?}");
        assert_eq!(harness.engine_ids().len(), 1);
        assert!(
            hooks::pending_events(Arc::clone(&harness.state.store))
                .await
                .expect("pending")
                .is_empty(),
            "a duplicate add journals nothing"
        );

        let webhook = webhook_server.join().expect("webhook server");
        assert!(webhook.contains("\"name\":\"kept.mkv\""), "{webhook}");
    });
}

#[tokio::test]
async fn config_reload_swaps_live_rules_and_keeps_the_old_one_on_error() {
    let paths = paths("reload");
    let config_path = paths.base.join("config.toml");
    let other_watch = paths.base.join("other");
    std::fs::create_dir_all(&other_watch).expect("other watch");
    std::fs::write(
        &config_path,
        format!(
            "[[autoadd]]\npath = {:?}\n\n[[hook]]\nevent = \"added\"\ncommand = \"/bin/true\"\n",
            paths.watch.display().to_string()
        ),
    )
    .expect("config v1");
    let config = ConfigStore::load(&config_path).expect("load");
    let harness = Harness::assemble(
        paths.clone(),
        config,
        Box::new(MemoryStore::new()),
        LabelState::default(),
        &[],
    )
    .await;

    harness
        .state
        .publish_event(NewEvent::new(EventKind::Added, 1, "one.mkv"))
        .await
        .expect("publish");
    assert_eq!(
        harness.state.run_pending_hooks().await.expect("hooks"),
        1,
        "the v1 hook runs"
    );

    std::fs::write(
        &config_path,
        format!(
            "[[autoadd]]\npath = {:?}\n\n[[hook]]\nevent = \"finished\"\ncommand = \"/bin/true\"\n",
            other_watch.display().to_string()
        ),
    )
    .expect("config v2");
    let revision = harness
        .state
        .config
        .reload(harness.state.config.revision())
        .expect("reload");
    let resolved = harness.state.config.resolved().expect("resolved");
    assert_eq!(resolved.autoadd[0].path, other_watch);
    assert_eq!(resolved.hook.len(), 1);
    assert_eq!(resolved.hook[0].event, EventKind::Finished);

    std::fs::write(&config_path, "this is not = toml [").expect("config v3");
    assert!(
        harness.state.config.reload(revision).is_err(),
        "a broken file is rejected"
    );
    assert_eq!(
        harness.state.config.revision(),
        revision,
        "the rejected reload keeps the running revision"
    );
    let resolved = harness.state.config.resolved().expect("resolved");
    assert_eq!(resolved.autoadd[0].path, other_watch);

    harness
        .state
        .publish_event(NewEvent::new(EventKind::Added, 2, "two.mkv"))
        .await
        .expect("publish");
    assert_eq!(
        harness.state.run_pending_hooks().await.expect("hooks"),
        0,
        "the old hook no longer matches"
    );
    harness
        .state
        .publish_event(NewEvent::new(EventKind::Finished, 3, "three.mkv"))
        .await
        .expect("publish");
    assert_eq!(
        harness.state.run_pending_hooks().await.expect("hooks"),
        1,
        "the new hook runs without a restart"
    );

    harness.write_torrent(&paths.watch, "old.torrent", "old.mkv", 10);
    harness.state.scan_once().await;
    let totals = harness.state.scan_once().await;
    assert_eq!(totals.added, 0, "the old watch dir is no longer scanned");
    harness.write_torrent(&other_watch, "new.torrent", "new.mkv", 10);
    harness.state.scan_once().await;
    let totals = harness.state.scan_once().await;
    assert_eq!(totals.added, 1, "{totals:?}");
}

#[tokio::test]
async fn a_hundred_watch_files_land_in_a_thousand_torrent_engine() {
    let paths = paths("scale");
    let config = ConfigStore::default();
    config
        .apply_toml(
            &format!(
                "[[autoadd]]\npath = {:?}\n",
                paths.watch.display().to_string()
            ),
            0,
        )
        .expect("config");

    let mut seed = Vec::with_capacity(1_000);
    for index in 0..1_000_u64 {
        seed.push(PersistedTorrent {
            id: index + 1,
            name: format!("seed-{index}.mkv"),
            save_path: paths.downloads.clone(),
            metainfo: torrent_bytes(&format!("seed-{index}.mkv"), 10),
            resume_data: None,
            paused_by_user: false,
            label: None,
        });
    }
    let harness = Harness::assemble(
        paths.clone(),
        config,
        Box::new(MemoryStore::new()),
        LabelState::default(),
        &seed,
    )
    .await;
    assert_eq!(harness.engine_ids().len(), 1_000);

    for index in 0..100 {
        harness.write_torrent(
            &paths.watch,
            &format!("{index:03}.torrent"),
            &format!("bulk-{index}.mkv"),
            10 + index,
        );
    }
    let started = Instant::now();
    let warm = harness.state.scan_once().await;
    let warm_elapsed = started.elapsed();
    assert_eq!(warm.waiting, 100, "{warm:?}");

    let started = Instant::now();
    let totals = harness.state.scan_once().await;
    let elapsed = started.elapsed();
    println!(
        "automation scale: 100 files into 1 000 torrents: warm {:.1} ms, add {:.1} ms",
        warm_elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000.0
    );
    assert_eq!(totals.added, 100, "{totals:?}");
    assert_eq!(harness.engine_ids().len(), 1_100);
    assert!(
        elapsed < Duration::from_secs(5),
        "100 adds took {elapsed:?}"
    );
}
