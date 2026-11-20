#![cfg(unix)]

mod support;

use std::path::PathBuf;
use std::time::Duration;

use ferrent_gui::actions::{self, TorrentAction};
use ferrent_gui::connection::{ListQuery, ServerConfig, connect, list};
use ferrent_protocol::v1::SortField;
use support::{start_server, temp_ca};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../engine/tests/fixtures/hello.torrent")
}

fn fixture_variant(dir: &std::path::Path, tag: u8) -> PathBuf {
    let mut bytes = std::fs::read(fixture()).expect("read fixture");
    bytes.push(tag);
    let path = dir.join(format!("hello-{tag}.torrent"));
    std::fs::write(&path, bytes).expect("write variant");
    path
}

async fn connect_to(
    server: &support::TestServer,
    ca: PathBuf,
) -> ferrent_protocol::client::AuthClient {
    let config = ServerConfig {
        server: format!("https://localhost:{}", server.addr.port()),
        ca,
        token: "secret".to_owned(),
    };
    connect(&config).await.expect("connect")
}

#[tokio::test]
async fn adds_pages_and_mutates_torrents() {
    let tls = ferrent_daemon::rpc::TlsMaterial::generate().expect("generate TLS");
    let (dir, ca) = temp_ca(&tls);
    let server = start_server(0, tls).await;
    let client = connect_to(&server, ca).await;

    let save_path = dir.join("downloads").display().to_string();
    let first = actions::add(&client, &fixture_variant(&dir, 1), &save_path, "", "")
        .await
        .expect("add first");
    let second = actions::add(&client, &fixture_variant(&dir, 2), &save_path, "", "")
        .await
        .expect("add second");
    let third = actions::add(&client, &fixture_variant(&dir, 3), &save_path, "", "")
        .await
        .expect("add third");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let page = list(
        &client,
        &ListQuery {
            filter: "hello".to_owned(),
            state: None,
            label: String::new(),
            sort: SortField::Id,
            descending: false,
            offset: 1,
            limit: 2,
        },
    )
    .await
    .expect("page");
    assert_eq!(page.total, 3);
    assert_eq!(page.torrents.len(), 2);
    assert_eq!(page.torrents[0].id, second);
    assert_eq!(page.torrents[1].id, third);
    assert_ne!(first, second);

    let errors = actions::apply(&client, TorrentAction::Pause, &[first]).await;
    assert!(errors.is_empty(), "{errors:?}");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let page = list(&client, &ListQuery::default()).await.expect("paused");
    let paused = page
        .torrents
        .iter()
        .find(|torrent| torrent.id == first)
        .expect("first torrent");
    assert_eq!(
        paused.state,
        ferrent_protocol::v1::TorrentState::Paused as i32
    );

    let errors = actions::apply(&client, TorrentAction::Resume, &[first]).await;
    assert!(errors.is_empty(), "{errors:?}");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let page = list(&client, &ListQuery::default()).await.expect("resumed");
    let resumed = page
        .torrents
        .iter()
        .find(|torrent| torrent.id == first)
        .expect("first torrent");
    assert_eq!(
        resumed.state,
        ferrent_protocol::v1::TorrentState::Downloading as i32
    );

    let errors = actions::apply(
        &client,
        TorrentAction::Remove {
            delete_files: false,
        },
        &[second],
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    tokio::time::sleep(Duration::from_millis(150)).await;
    let page = list(&client, &ListQuery::default()).await.expect("removed");
    assert_eq!(page.total, 2);
    assert!(page.torrents.iter().all(|torrent| torrent.id != second));

    let errors = actions::apply(
        &client,
        TorrentAction::Remove {
            delete_files: false,
        },
        &[999],
    )
    .await;
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("NotFound") || errors[0].contains("not found"));

    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
