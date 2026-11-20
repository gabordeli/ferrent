#![cfg(unix)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::{MemoryStore, StateStore};
use ferrent_engine::{MockEngine, TorrentEngine};
use ferrent_protocol::client::connect;
use ferrent_protocol::v1::watch_response::Event as WatchEvent;
use ferrent_protocol::v1::{
    AddTorrentRequest, GetConfigRequest, GetTorrentRequest, ListTorrentsRequest,
    PauseTorrentRequest, RemoveTorrentRequest, ResumeTorrentRequest, SetConfigRequest, SortField,
    TorrentState, WatchRequest,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

struct TestServer {
    addr: SocketAddr,
    tls: TlsMaterial,
    server: JoinHandle<()>,
    mirror: JoinHandle<()>,
}

impl TestServer {
    fn uri(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
    }

    fn shutdown(self) {
        self.server.abort();
        self.mirror.abort();
    }
}

async fn start_server(token: &str) -> TestServer {
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
        token,
        std::path::PathBuf::from("."),
        std::path::PathBuf::from("/tmp"),
    ));

    let tls = TlsMaterial::generate().expect("generate TLS material");
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
        tls,
        server,
        mirror: mirror_task,
    }
}

fn list_request() -> ListTorrentsRequest {
    ListTorrentsRequest {
        filter: String::new(),
        sort: SortField::Id as i32,
        descending: false,
        offset: 0,
        limit: 10,
        state_filter: 0,
    }
}

#[tokio::test]
async fn tls_auth_and_crud() {
    let server = start_server("secret").await;
    let mut client = connect(&server.uri(), &server.tls.cert_pem, "secret")
        .await
        .expect("connect");

    let added = client
        .add_torrent(AddTorrentRequest {
            name: "test".to_owned(),
            metainfo: Vec::new(),
            save_path: "/tmp".to_owned(),
            total_bytes: 123,
        })
        .await
        .expect("add")
        .into_inner();
    assert!(!added.duplicate);

    let duplicate = client
        .add_torrent(AddTorrentRequest {
            name: "test".to_owned(),
            metainfo: Vec::new(),
            save_path: "/tmp".to_owned(),
            total_bytes: 123,
        })
        .await
        .expect("duplicate add")
        .into_inner();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.id, added.id);

    tokio::time::sleep(Duration::from_millis(150)).await;

    let list = client
        .list_torrents(list_request())
        .await
        .expect("list")
        .into_inner();
    assert_eq!(list.total, 1);
    assert_eq!(list.torrents.len(), 1);
    assert_eq!(list.torrents[0].name, "test");
    assert_eq!(list.torrents[0].infohash.len(), 40);

    client
        .pause_torrent(PauseTorrentRequest { id: added.id })
        .await
        .expect("pause");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let paused = client
        .get_torrent(GetTorrentRequest { id: added.id })
        .await
        .expect("get paused")
        .into_inner()
        .torrent
        .expect("torrent");
    assert_eq!(paused.state, TorrentState::Paused as i32);
    assert_ne!(paused.pause_reasons & 1, 0, "manual pause must be visible");

    client
        .resume_torrent(ResumeTorrentRequest { id: added.id })
        .await
        .expect("resume");
    let resumed = client
        .get_torrent(GetTorrentRequest { id: added.id })
        .await
        .expect("get resumed")
        .into_inner()
        .torrent
        .expect("torrent");
    assert_eq!(resumed.pause_reasons, 0);
    client
        .remove_torrent(RemoveTorrentRequest {
            id: added.id,
            delete_files: false,
        })
        .await
        .expect("remove");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let error = client
        .get_torrent(GetTorrentRequest { id: added.id })
        .await
        .expect_err("removed torrent");
    assert_eq!(error.code(), tonic::Code::NotFound);

    server.shutdown();
}

#[tokio::test]
async fn wrong_token_is_unauthenticated() {
    let server = start_server("secret").await;
    let mut client = connect(&server.uri(), &server.tls.cert_pem, "wrong")
        .await
        .expect("connect");

    let error = client
        .list_torrents(list_request())
        .await
        .expect_err("wrong token");
    assert_eq!(error.code(), tonic::Code::Unauthenticated);

    server.shutdown();
}

#[tokio::test]
async fn wrong_ca_fails_the_handshake() {
    let server = start_server("secret").await;
    let other = TlsMaterial::generate().expect("other TLS material");

    let result = connect(&server.uri(), &other.cert_pem, "secret").await;
    assert!(result.is_err(), "a different CA must fail");
    let error = result
        .err()
        .unwrap_or_else(|| panic!("a different CA unexpectedly connected"));
    assert!(!error.to_string().is_empty());

    server.shutdown();
}

#[tokio::test]
async fn watch_streams_added_and_stays_idle() {
    let server = start_server("secret").await;
    let mut client = connect(&server.uri(), &server.tls.cert_pem, "secret")
        .await
        .expect("connect");

    let mut stream = client
        .watch(WatchRequest {})
        .await
        .expect("watch")
        .into_inner();

    client
        .add_torrent(AddTorrentRequest {
            name: "watched".to_owned(),
            metainfo: Vec::new(),
            save_path: "/tmp".to_owned(),
            total_bytes: 1,
        })
        .await
        .expect("add");

    let response = tokio::time::timeout(Duration::from_secs(1), stream.message())
        .await
        .expect("watch event timeout")
        .expect("watch stream");
    let event = response.expect("watch message");
    match event.event {
        Some(WatchEvent::Added(torrent)) => assert_eq!(torrent.name, "watched"),
        other => panic!("unexpected watch event: {other:?}"),
    }

    let idle = tokio::time::timeout(Duration::from_millis(500), stream.message()).await;
    assert!(idle.is_err(), "expected no idle event, got {idle:?}");

    server.shutdown();
}

#[tokio::test]
async fn config_roundtrip_and_revision_conflict() {
    let server = start_server("secret").await;
    let mut client = connect(&server.uri(), &server.tls.cert_pem, "secret")
        .await
        .expect("connect");

    let initial = client
        .get_config(GetConfigRequest {})
        .await
        .expect("get config")
        .into_inner();
    assert_eq!(initial.revision, 0);
    assert!(initial.toml.is_empty());

    let text = "# keep\nbind = \"127.0.0.1:50051\"\n[session]\ndownload_rate_limit = 1024\n";
    let updated = client
        .set_config(SetConfigRequest {
            toml: text.to_owned(),
            revision: initial.revision,
        })
        .await
        .expect("set config")
        .into_inner();
    assert_eq!(updated.revision, 1);

    let fetched = client
        .get_config(GetConfigRequest {})
        .await
        .expect("get config")
        .into_inner();
    assert_eq!(fetched.revision, 1);
    assert_eq!(fetched.toml, text);

    let stale = client
        .set_config(SetConfigRequest {
            toml: "bind = \"127.0.0.1:50052\"\n".to_owned(),
            revision: 0,
        })
        .await
        .expect_err("stale revision");
    assert_eq!(stale.code(), tonic::Code::Aborted);

    let invalid = client
        .set_config(SetConfigRequest {
            toml: "[session]\nnope = 1\n".to_owned(),
            revision: 1,
        })
        .await
        .expect_err("invalid config");
    assert_eq!(invalid.code(), tonic::Code::InvalidArgument);

    let after = client
        .get_config(GetConfigRequest {})
        .await
        .expect("get config")
        .into_inner();
    assert_eq!(after.revision, 1);
    assert_eq!(after.toml, text);

    server.shutdown();
}

#[tokio::test]
async fn watch_streams_config_changes() {
    let server = start_server("secret").await;
    let mut client = connect(&server.uri(), &server.tls.cert_pem, "secret")
        .await
        .expect("connect");

    let mut stream = client
        .watch(WatchRequest {})
        .await
        .expect("watch")
        .into_inner();

    client
        .set_config(SetConfigRequest {
            toml: "bind = \"127.0.0.1:50051\"\n".to_owned(),
            revision: 0,
        })
        .await
        .expect("set config");

    let response = tokio::time::timeout(Duration::from_secs(1), stream.message())
        .await
        .expect("config event timeout")
        .expect("watch stream");
    let event = response.expect("watch message");
    match event.event {
        Some(WatchEvent::ConfigChanged(change)) => assert_eq!(change.revision, 1),
        other => panic!("unexpected watch event: {other:?}"),
    }

    server.shutdown();
}
