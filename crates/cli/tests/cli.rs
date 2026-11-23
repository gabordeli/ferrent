#![cfg(unix)]
#![allow(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::LabelState;
use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::{MemoryStore, StateStore};
use ferrent_engine::{MockEngine, TorrentEngine};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

struct TestServer {
    addr: SocketAddr,
    dir: PathBuf,
    server: JoinHandle<()>,
    mirror: JoinHandle<()>,
}

impl TestServer {
    fn uri(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
    }

    fn cert_path(&self) -> PathBuf {
        self.dir.join("ca.pem")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("cli.toml")
    }

    fn torrent_path(&self) -> PathBuf {
        self.dir.join("test.torrent")
    }

    fn shutdown(self) {
        self.server.abort();
        self.mirror.abort();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start_server() -> TestServer {
    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    let mock = MockEngine::new();
    let events = mock.subscribe();
    let engine: Box<dyn TorrentEngine> = Box::new(mock);
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(64);
    let mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let store: Box<dyn StateStore> = Box::new(MemoryStore::new());
    let state = Arc::new(AppState::new(
        engine,
        mirror,
        watch,
        store,
        ConfigStore::default(),
        TorrentControl::default(),
        LabelState::default(),
        "secret",
        PathBuf::from("."),
    ));

    let tls = TlsMaterial::generate().expect("generate TLS material");
    let dir = std::env::temp_dir().join(format!(
        "ferrent-cli-test-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    std::fs::write(dir.join("ca.pem"), &tls.cert_pem).expect("write CA");
    std::fs::write(dir.join("test.torrent"), b"dummy metainfo").expect("write torrent");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server_tls = tls.clone();
    let server = tokio::spawn(async move {
        serve(listener, state, &server_tls).await.expect("serve");
    });

    TestServer {
        addr,
        dir,
        server,
        mirror: mirror_task,
    }
}

async fn run_cli(server: &TestServer, args: &[&str]) -> std::process::Output {
    run_cli_with_token(server, "secret", args).await
}

async fn run_cli_with_token(
    server: &TestServer,
    token: &str,
    args: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .env("FERRENT_BUGSINK_DSN", "")
        .arg("--server")
        .arg(server.uri())
        .arg("--ca")
        .arg(server.cert_path())
        .arg("--config")
        .arg(server.config_path())
        .arg("--token")
        .arg(token)
        .args(args)
        .output()
        .await
        .expect("run ferrent-cli")
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn cli_crud_json_and_exit_codes() {
    let server = start_server().await;
    let torrent = server.torrent_path();
    let torrent = torrent.to_str().expect("torrent path");

    let add = run_cli(
        &server,
        &[
            "--json",
            "add",
            torrent,
            "--name",
            "test",
            "--save-path",
            "/tmp",
        ],
    )
    .await;
    assert!(add.status.success(), "add failed: {}", stderr(&add));
    let added: Value = serde_json::from_slice(&add.stdout).expect("add JSON");
    let id = added["id"].as_u64().expect("torrent id");
    let id_arg = id.to_string();

    tokio::time::sleep(Duration::from_millis(150)).await;
    let list = run_cli(&server, &["--json", "list"]).await;
    assert!(list.status.success(), "list failed: {}", stderr(&list));
    let listed: Value = serde_json::from_slice(&list.stdout).expect("list JSON");
    assert_eq!(listed["total"].as_u64(), Some(1));
    assert_eq!(listed["torrents"][0]["name"].as_str(), Some("test"));

    let show = run_cli(&server, &["--json", "show", &id_arg]).await;
    assert!(show.status.success(), "show failed: {}", stderr(&show));
    let shown: Value = serde_json::from_slice(&show.stdout).expect("show JSON");
    assert_eq!(shown["id"].as_u64(), Some(id));

    for args in [
        vec!["pause", id_arg.as_str()],
        vec!["resume", id_arg.as_str()],
        vec!["remove", id_arg.as_str()],
    ] {
        let output = run_cli(&server, &args).await;
        assert!(
            output.status.success(),
            "{} failed: {}",
            args.join(" "),
            stderr(&output)
        );
    }

    tokio::time::sleep(Duration::from_millis(150)).await;
    let list = run_cli(&server, &["--json", "list"]).await;
    assert!(list.status.success());
    let listed: Value = serde_json::from_slice(&list.stdout).expect("list JSON");
    assert_eq!(listed["total"].as_u64(), Some(0));

    let missing = run_cli(&server, &["--json", "show", &id_arg]).await;
    assert_eq!(missing.status.code(), Some(3));
    assert!(stderr(&missing).contains("not found"));

    let wrong_token = run_cli_with_token(&server, "wrong", &["--json", "list"]).await;
    assert_eq!(wrong_token.status.code(), Some(4));

    let help = Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .arg("--help")
        .output()
        .await
        .expect("run help");
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    for command in ["add", "list", "show", "pause", "resume", "remove", "watch"] {
        assert!(help_text.contains(command), "help is missing {command}");
    }

    server.shutdown();
}

#[tokio::test]
async fn watch_receives_added_event() {
    let server = start_server().await;
    let child = Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .arg("--server")
        .arg(server.uri())
        .arg("--ca")
        .arg(server.cert_path())
        .arg("--config")
        .arg(server.config_path())
        .arg("--token")
        .arg("secret")
        .args(["--json", "watch", "--count", "1"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn watch");

    // The CLI installs its Ctrl-C handler after connecting; on a loaded
    // machine 300 ms is not enough for that, and SIGINT would kill it.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let torrent = server.torrent_path();
    let add = run_cli(
        &server,
        &[
            "--json",
            "add",
            torrent.to_str().expect("torrent path"),
            "--name",
            "watched",
        ],
    )
    .await;
    assert!(add.status.success(), "add failed: {}", stderr(&add));

    // Generous: the debug build and a loaded machine can starve the child.
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("watch timed out")
        .expect("wait watch");
    assert!(output.status.success(), "watch failed: {}", stderr(&output));
    let line = String::from_utf8_lossy(&output.stdout);
    let event: Value = serde_json::from_str(line.trim()).expect("watch JSON");
    assert_eq!(event["event"].as_str(), Some("added"));
    assert_eq!(event["torrent"]["name"].as_str(), Some("watched"));

    server.shutdown();
}

#[tokio::test]
async fn cli_config_get_and_set() {
    let server = start_server().await;

    let get = run_cli(&server, &["--json", "config", "get"]).await;
    assert!(get.status.success(), "config get failed: {}", stderr(&get));
    let initial: Value = serde_json::from_slice(&get.stdout).expect("get JSON");
    assert_eq!(initial["revision"].as_u64(), Some(0));
    assert_eq!(initial["toml"].as_str(), Some(""));

    let path = server.dir.join("daemon.toml");
    let text = "# keep\nbind = \"127.0.0.1:50051\"\n";
    std::fs::write(&path, text).expect("write config");

    let set = run_cli(
        &server,
        &[
            "--json",
            "config",
            "set",
            path.to_str().expect("config path"),
        ],
    )
    .await;
    assert!(set.status.success(), "config set failed: {}", stderr(&set));
    let updated: Value = serde_json::from_slice(&set.stdout).expect("set JSON");
    assert_eq!(updated["revision"].as_u64(), Some(1));

    let get = run_cli(&server, &["--json", "config", "get"]).await;
    assert!(get.status.success(), "config get failed: {}", stderr(&get));
    let fetched: Value = serde_json::from_slice(&get.stdout).expect("get JSON");
    assert_eq!(fetched["revision"].as_u64(), Some(1));
    assert_eq!(fetched["toml"].as_str(), Some(text));

    server.shutdown();
}

#[tokio::test]
async fn watch_ctrl_c_exits_zero() {
    let server = start_server().await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .arg("--server")
        .arg(server.uri())
        .arg("--ca")
        .arg(server.cert_path())
        .arg("--config")
        .arg(server.config_path())
        .arg("--token")
        .arg("secret")
        .arg("watch")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn watch");

    // The CLI installs its Ctrl-C handler after connecting; on a loaded
    // machine 300 ms is not enough for that, and SIGINT would kill it.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let pid = child.id().expect("watch pid") as i32;
    let result = unsafe { libc::kill(pid, libc::SIGINT) };
    assert_eq!(result, 0, "SIGINT delivery failed");

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("watch did not exit")
        .expect("wait watch");
    assert_eq!(status.code(), Some(0));

    server.shutdown();
}

#[tokio::test]
async fn version_matches_the_workspace_and_bad_transport_is_clear() {
    let raw = Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .arg("--version")
        .output()
        .await
        .expect("run ferrent-cli");
    assert!(raw.status.success());
    assert_eq!(
        String::from_utf8_lossy(&raw.stdout).trim(),
        format!("ferrent-cli {}", env!("CARGO_PKG_VERSION"))
    );

    let server = start_server().await;
    let down = Command::new(env!("CARGO_BIN_EXE_ferrent-cli"))
        .env("FERRENT_BUGSINK_DSN", "")
        .args([
            "--server",
            "https://127.0.0.1:1",
            "--ca",
            server.cert_path().to_str().expect("ca"),
            "--config",
            server.config_path().to_str().expect("config"),
            "--token",
            "secret",
            "status",
        ])
        .output()
        .await
        .expect("run ferrent-cli");
    assert_eq!(down.status.code(), Some(5), "{}", stderr(&down));
    let message = stderr(&down);
    assert!(
        message.contains("cannot connect to https://127.0.0.1:1"),
        "{message}"
    );
    assert!(!message.contains("panicked"), "{message}");
    server.shutdown();
}

#[tokio::test]
async fn every_read_command_answers_json() {
    let server = start_server().await;
    let state = server.dir.join("deluge").join("state");
    std::fs::create_dir_all(&state).expect("state dir");
    let state_arg = state.to_str().expect("state path").to_owned();

    let commands: Vec<Vec<String>> = vec![
        vec!["--json".into(), "status".into()],
        vec!["--json".into(), "stats".into()],
        vec!["--json".into(), "disks".into()],
        vec!["--json".into(), "list".into()],
        vec!["--json".into(), "rss".into()],
        vec!["--json".into(), "autoadd".into()],
        vec!["--json".into(), "autoadd".into(), "--scan".into()],
        vec!["--json".into(), "hooks".into()],
        vec!["--json".into(), "labels".into()],
        vec!["--json".into(), "labels".into(), "list".into()],
        vec!["--json".into(), "blocklist".into()],
        vec!["--json".into(), "notify".into(), "status".into()],
        vec!["--json".into(), "config".into(), "get".into()],
        vec!["--json".into(), "servers".into()],
        vec![
            "--json".into(),
            "import-deluge".into(),
            "--state-dir".into(),
            state_arg,
            "--dry-run".into(),
        ],
    ];
    for args in &commands {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = run_cli(&server, &borrowed).await;
        assert!(
            output.status.success(),
            "{} failed: {}",
            borrowed.join(" "),
            stderr(&output)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{} did not print JSON ({error}): {}",
                borrowed.join(" "),
                String::from_utf8_lossy(&output.stdout)
            )
        });
    }
    server.shutdown();
}

#[tokio::test]
async fn every_torrent_command_runs() {
    let server = start_server().await;
    let torrent = server.torrent_path();
    let torrent = torrent.to_str().expect("torrent path").to_owned();
    let add = run_cli(
        &server,
        &[
            "--json",
            "add",
            &torrent,
            "--name",
            "commands",
            "--save-path",
            "/tmp",
        ],
    )
    .await;
    assert!(add.status.success(), "add failed: {}", stderr(&add));
    let added: Value = serde_json::from_slice(&add.stdout).expect("add JSON");
    let id = added["id"].as_u64().expect("id").to_string();

    let commands: Vec<Vec<String>> = vec![
        vec!["peers".into(), id.clone()],
        vec!["files".into(), id.clone()],
        vec![
            "rename-file".into(),
            id.clone(),
            "0".into(),
            "renamed.bin".into(),
        ],
        vec!["trackers".into(), id.clone()],
        vec![
            "trackers".into(),
            id.clone(),
            "--add".into(),
            "udp://tracker.example:6969".into(),
        ],
        vec!["webseeds".into(), id.clone()],
        vec![
            "webseeds".into(),
            id.clone(),
            "--set".into(),
            "https://mirror.example/file".into(),
        ],
        vec![
            "limits".into(),
            id.clone(),
            "--download".into(),
            "1048576".into(),
        ],
        vec!["limits".into(), id.clone(), "--clear".into()],
        vec!["set".into(), id.clone(), "--sequential".into()],
        vec!["set".into(), id.clone(), "--no-super-seeding".into()],
        vec!["recheck".into(), id.clone()],
        vec!["pause".into(), id.clone()],
        vec!["resume".into(), id.clone()],
        vec!["move".into(), id.clone()],
        vec!["move".into(), id.clone(), "--down".into()],
    ];
    for args in &commands {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = run_cli(&server, &borrowed).await;
        assert!(
            output.status.success(),
            "{} failed: {}",
            borrowed.join(" "),
            stderr(&output)
        );
    }

    let peers = run_cli(&server, &["--json", "peers", &id]).await;
    assert!(peers.status.success(), "peers failed: {}", stderr(&peers));

    let create_src = server.dir.join("payload.bin");
    std::fs::write(&create_src, vec![0_u8; 40 * 1024]).expect("payload");
    let created = run_cli(
        &server,
        &[
            "--json",
            "create",
            create_src.to_str().expect("payload path"),
            "--output",
            server.dir.join("built.torrent").to_str().expect("out"),
        ],
    )
    .await;
    assert!(
        created.status.success(),
        "create failed: {}",
        stderr(&created)
    );

    let removed = run_cli(&server, &["--json", "remove", &id]).await;
    assert!(
        removed.status.success(),
        "remove failed: {}",
        stderr(&removed)
    );
    server.shutdown();
}

#[tokio::test]
async fn labels_round_trip_with_an_assignment() {
    let server = start_server().await;
    let created = run_cli(
        &server,
        &[
            "--json",
            "labels",
            "create",
            "movies",
            "--color",
            "#3ba55d",
            "--move-completed",
            "/mnt/usbhdd1/library",
            "--download-limit",
            "1048576",
        ],
    )
    .await;
    assert!(
        created.status.success(),
        "create failed: {}",
        stderr(&created)
    );

    let listed = run_cli(&server, &["--json", "labels", "list"]).await;
    assert!(listed.status.success(), "list failed: {}", stderr(&listed));
    let body: Value = serde_json::from_slice(&listed.stdout).expect("labels JSON");
    let label = &body["labels"][0];
    assert_eq!(label["name"].as_str(), Some("movies"));
    assert_eq!(label["color"].as_str(), Some("#3ba55d"));
    assert_eq!(label["download_rate_limit"].as_u64(), Some(1_048_576));

    let updated = run_cli(
        &server,
        &["--json", "labels", "update", "movies", "--color", "#ff0000"],
    )
    .await;
    assert!(
        updated.status.success(),
        "update failed: {}",
        stderr(&updated)
    );

    let torrent = server.torrent_path();
    let add = run_cli(
        &server,
        &[
            "--json",
            "add",
            torrent.to_str().expect("path"),
            "--name",
            "l",
        ],
    )
    .await;
    let added: Value = serde_json::from_slice(&add.stdout).expect("add JSON");
    let id = added["id"].as_u64().expect("id").to_string();

    let assigned = run_cli(&server, &["--json", "labels", "set", &id, "movies"]).await;
    assert!(
        assigned.status.success(),
        "set failed: {}",
        stderr(&assigned)
    );
    let body: Value = serde_json::from_slice(&assigned.stdout).expect("set JSON");
    assert_eq!(body["label"].as_str(), Some("movies"));

    let cleared = run_cli(&server, &["--json", "labels", "set", &id, ""]).await;
    assert!(
        cleared.status.success(),
        "clear failed: {}",
        stderr(&cleared)
    );

    let removed = run_cli(&server, &["--json", "labels", "delete", "movies"]).await;
    assert!(
        removed.status.success(),
        "delete failed: {}",
        stderr(&removed)
    );
    let body: Value = serde_json::from_slice(&removed.stdout).expect("delete JSON");
    assert_eq!(body["removed"].as_bool(), Some(true));
    server.shutdown();
}

#[tokio::test]
async fn servers_are_stored_in_the_cli_config() {
    let server = start_server().await;
    let added = run_cli(
        &server,
        &[
            "--json",
            "servers",
            "add",
            "lab",
            "--server",
            "https://192.168.2.28:50051",
            "--ca",
            "/tmp/ca.pem",
            "--token",
            "lab-token",
            "--use",
        ],
    )
    .await;
    assert!(added.status.success(), "add failed: {}", stderr(&added));
    let text = std::fs::read_to_string(server.config_path()).expect("cli.toml");
    assert!(text.contains("[[servers]]"), "{text}");
    assert!(text.contains("active = \"lab\""), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(server.config_path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the token file must stay private");
    }

    let listed = run_cli(&server, &["--json", "servers", "list"]).await;
    assert!(listed.status.success(), "list failed: {}", stderr(&listed));
    let body: Value = serde_json::from_slice(&listed.stdout).expect("servers JSON");
    assert_eq!(body["active"].as_str(), Some("lab"));
    assert_eq!(body["servers"][0]["token_set"].as_bool(), Some(true));

    let used = run_cli(&server, &["servers", "use", "lab"]).await;
    assert!(used.status.success(), "use failed: {}", stderr(&used));

    let removed = run_cli(&server, &["--json", "servers", "remove", "lab"]).await;
    assert!(
        removed.status.success(),
        "remove failed: {}",
        stderr(&removed)
    );
    let body: Value = serde_json::from_slice(&removed.stdout).expect("remove JSON");
    assert_eq!(body["removed"].as_bool(), Some(true));
    let text = std::fs::read_to_string(server.config_path()).expect("cli.toml");
    assert!(!text.contains("[[servers]]"), "{text}");
    server.shutdown();
}

#[tokio::test]
async fn rss_preview_reads_a_local_feed() {
    use tokio::io::AsyncWriteExt;

    let server = start_server().await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind feed listener");
    let addr = listener.local_addr().expect("feed addr");
    let feed = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let body = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
<item><title>Some.Release.1080p</title><enclosure url="http://example/a.torrent" type="application/x-bittorrent"/></item>
<item><title>Some.Release.720p</title><enclosure url="http://example/b.torrent" type="application/x-bittorrent"/></item>
</channel></rss>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    let url = format!("http://{addr}/feed.xml");
    let output = run_cli(
        &server,
        &["--json", "rss", "--preview", &url, "--filter", "1080p"],
    )
    .await;
    assert!(
        output.status.success(),
        "preview failed: {}",
        stderr(&output)
    );
    let body: Value = serde_json::from_slice(&output.stdout).expect("preview JSON");
    assert_eq!(body["total"].as_u64(), Some(2));
    let matched: Vec<&Value> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["matched"] == true)
        .collect();
    assert_eq!(matched.len(), 1, "{body}");
    assert!(
        matched[0]["title"]
            .as_str()
            .is_some_and(|title| title.contains("1080p"))
    );
    feed.abort();
    server.shutdown();
}

#[tokio::test]
async fn cli_status_reports_the_daemon_identity() {
    let server = start_server().await;
    let output = run_cli(&server, &["--json", "status"]).await;
    assert!(output.status.success(), "{}", stderr(&output));
    let body: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        body["daemon_version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(body["server"].as_str(), Some(server.uri().as_str()));
    assert!(body["uptime_secs"].as_i64().expect("uptime") >= 0);
    let fingerprint = body["fingerprint"].as_str().expect("fingerprint");
    assert_eq!(fingerprint.split(':').count(), 32, "{fingerprint}");
    assert_eq!(body["torrent_count"].as_u64(), Some(0));

    let human = run_cli(&server, &["status"]).await;
    assert!(human.status.success(), "{}", stderr(&human));
    let text = String::from_utf8_lossy(&human.stdout).into_owned();
    assert!(text.contains("daemon:      ferrent "), "{text}");
    assert!(text.contains("tls sha256:  "), "{text}");
    server.shutdown();
}

#[tokio::test]
async fn cli_disks_reports_the_store_volume() {
    let server = start_server().await;
    let output = run_cli(&server, &["--json", "disks"]).await;
    assert!(output.status.success(), "{}", stderr(&output));
    let body: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(body["enabled"], false);
    let disks = body["disks"].as_array().expect("disks array");
    assert!(!disks.is_empty(), "{body}");
    let first = &disks[0];
    assert!(first["mount"].as_str().expect("mount").starts_with('/'));
    assert!(first["free_bytes"].as_u64().expect("free") > 0);
    assert!(first["zones"].is_array());
    server.shutdown();
}
