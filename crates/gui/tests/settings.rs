#![cfg(unix)]

mod support;

use ferrent_gui::connection::{self, ServerConfig, connect};
use ferrent_protocol::v1::Label;
use support::start_server;

async fn client(
    tag: &str,
) -> (
    ferrent_protocol::client::AuthClient,
    support::TestServer,
    std::path::PathBuf,
) {
    let tls = ferrent_daemon::rpc::TlsMaterial::generate().expect("generate TLS");
    // A directory per test: the shared `temp_ca` helper would race between
    // tests running in parallel.
    let dir =
        std::env::temp_dir().join(format!("ferrent-gui-settings-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let ca_path = dir.join("ca.pem");
    std::fs::write(&ca_path, &tls.cert_pem).expect("write CA");

    let server = start_server(0, tls).await;
    let config = ServerConfig {
        server: format!("https://localhost:{}", server.addr.port()),
        ca: ca_path,
        token: "secret".to_owned(),
    };
    let client = connect(&config).await.expect("connect");
    (client, server, dir)
}

#[tokio::test]
async fn config_round_trip_and_stale_revision() {
    let (client, server, dir) = client("config").await;

    let snapshot = connection::config(&client).await.expect("get config");
    assert_eq!(snapshot.revision, 0);
    assert!(snapshot.toml.is_empty());

    let document = "[session]\ndownload_rate_limit = 2048\n";
    let revision = connection::set_config(&client, document, 0)
        .await
        .expect("set config");
    assert_eq!(revision, 1);

    let snapshot = connection::config(&client).await.expect("get config");
    assert_eq!(snapshot.revision, 1);
    assert!(snapshot.toml.contains("2048"), "{}", snapshot.toml);

    let stale = connection::set_config(&client, document, 0).await;
    assert!(stale.is_err(), "a stale revision must be rejected");
    let snapshot = connection::config(&client).await.expect("get config");
    assert_eq!(snapshot.revision, 1, "a rejected write keeps the revision");

    let invalid = connection::set_config(&client, "[session]\nnope = 1\n", 1).await;
    assert!(invalid.is_err(), "unknown keys must be rejected");

    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn label_crud_over_rpc() {
    let (client, server, dir) = client("labels").await;

    assert!(
        connection::labels(&client)
            .await
            .expect("list labels")
            .is_empty()
    );

    let created = connection::save_label(
        &client,
        Label {
            name: "movies".to_owned(),
            color: "#112233".to_owned(),
            download_rate_limit: 1024,
            ..Label::default()
        },
        true,
    )
    .await
    .expect("create label");
    assert_eq!(created.name, "movies");

    let labels = connection::labels(&client).await.expect("list labels");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].torrent_count, 0);
    assert_eq!(labels[0].label.as_ref().expect("label").color, "#112233");

    let updated = connection::save_label(
        &client,
        Label {
            name: "movies".to_owned(),
            color: "#445566".to_owned(),
            ..Label::default()
        },
        false,
    )
    .await
    .expect("update label");
    assert_eq!(updated.color, "#445566");
    let labels = connection::labels(&client).await.expect("list labels");
    assert_eq!(labels.len(), 1, "update must not create a second label");

    let invalid = connection::save_label(
        &client,
        Label {
            name: "with space".to_owned(),
            ..Label::default()
        },
        true,
    )
    .await;
    assert!(invalid.is_err(), "invalid names must be rejected");

    let cleared = connection::delete_label(&client, "movies")
        .await
        .expect("delete label");
    assert_eq!(cleared, 0);
    assert!(
        connection::labels(&client)
            .await
            .expect("list labels")
            .is_empty()
    );

    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
