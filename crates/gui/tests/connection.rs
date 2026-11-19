#![cfg(unix)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{MockEngine, TorrentEngine};
use ferrent_gui::connection::{ServerConfig, connect, list};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

struct TestServer {
    addr: SocketAddr,
    server: JoinHandle<()>,
    mirror: JoinHandle<()>,
}

impl TestServer {
    fn shutdown(self) {
        self.server.abort();
        self.mirror.abort();
    }
}

async fn start_server(port: u16, tls: TlsMaterial) -> TestServer {
    let mock = MockEngine::new();
    let events = mock.subscribe();
    let engine: Box<dyn TorrentEngine> = Box::new(mock);
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(64);
    let mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let store = Box::new(MemoryStore::new());
    let state = Arc::new(AppState::new(engine, mirror, watch, store, "secret"));

    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server_tls = tls;
    let server = tokio::spawn(async move {
        serve(listener, state, &server_tls).await.expect("serve");
    });

    TestServer {
        addr,
        server,
        mirror: mirror_task,
    }
}

fn temp_ca(tls: &TlsMaterial) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ferrent-gui-connection-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let ca = dir.join("ca.pem");
    std::fs::write(&ca, &tls.cert_pem).expect("write CA");
    (dir, ca)
}

#[tokio::test]
async fn reconnects_after_daemon_restart() {
    let tls = TlsMaterial::generate().expect("generate TLS");
    let (dir, ca_path) = temp_ca(&tls);

    let first = start_server(0, tls.clone()).await;
    let port = first.addr.port();
    let config = ServerConfig {
        server: format!("https://localhost:{port}"),
        ca: ca_path,
        token: "secret".to_owned(),
    };

    let client = connect(&config).await.expect("connect");
    let torrents = list(&client).await.expect("list");
    assert!(torrents.is_empty());
    drop(client);

    first.shutdown();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        connect(&config).await.is_err(),
        "server should be down after shutdown"
    );

    let second = start_server(port, tls).await;
    let client = connect(&config).await.expect("reconnect");
    let torrents = list(&client).await.expect("list after restart");
    assert!(torrents.is_empty());
    second.shutdown();

    let _ = std::fs::remove_dir_all(dir);
}
