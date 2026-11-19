#![cfg(unix)]
#![allow(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
        "secret",
        PathBuf::from("."),
        PathBuf::from("/tmp"),
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

    tokio::time::sleep(Duration::from_millis(300)).await;
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

    let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
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

    tokio::time::sleep(Duration::from_millis(300)).await;
    let pid = child.id().expect("watch pid") as i32;
    let result = unsafe { libc::kill(pid, libc::SIGINT) };
    assert_eq!(result, 0, "SIGINT delivery failed");

    let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .expect("watch did not exit")
        .expect("wait watch");
    assert_eq!(status.code(), Some(0));

    server.shutdown();
}
