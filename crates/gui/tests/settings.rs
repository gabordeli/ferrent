#![cfg(unix)]

mod support;

use ferrent_gui::connection::{self, ServerConfig, connect};
use ferrent_protocol::v1::{AddTorrentRequest, Label};
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

#[tokio::test]
async fn auto_add_status_and_manual_scan() {
    let (client, server, dir) = client("autoadd").await;

    connection::save_label(
        &client,
        Label {
            name: "movies".to_owned(),
            ..Label::default()
        },
        true,
    )
    .await
    .expect("create label");

    let watch = dir.join("watch");
    std::fs::create_dir_all(&watch).expect("watch dir");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"d4:infod6:lengthi9e4:name8:demo.mkvee");
    std::fs::write(watch.join("demo.torrent"), bytes).expect("write torrent");

    let toml = format!(
        "[[autoadd]]\npath = {:?}\nlabel = \"movies\"\n",
        watch.display().to_string()
    );
    let revision = connection::set_config(&client, &toml, 0)
        .await
        .expect("set config");
    assert_eq!(revision, 1);

    let status = connection::auto_add_status(&client).await.expect("status");
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].path, watch.display().to_string());
    assert_eq!(status[0].label, "movies");
    assert_eq!(status[0].added, 0);

    let totals = connection::scan_auto_add(&client, "").await.expect("scan");
    assert_eq!(totals.added, 1, "{totals:?}");
    assert!(!watch.join("demo.torrent").exists(), "file was consumed");

    let status = connection::auto_add_status(&client).await.expect("status");
    assert_eq!(status[0].added, 1);
    assert!(status[0].last_scan_unix > 0);
    assert!(status[0].last_error.is_empty(), "{:?}", status[0]);

    let missing = connection::scan_auto_add(&client, "/nope").await;
    assert!(missing.is_err(), "an unknown watch path is rejected");

    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn hook_runs_appear_in_the_status() {
    let (client, server, dir) = client("hooks").await;

    let config =
        "[[hook]]\nevent = \"added\"\ncommand = \"/bin/sh\"\nargs = [\"-c\", \"exit 0\"]\n";
    connection::set_config(&client, config, 0)
        .await
        .expect("set config");

    let mut metainfo = Vec::new();
    metainfo.extend_from_slice(b"d4:infod6:lengthi5e4:name4:hookee");
    let mut add_client = client.clone();
    add_client
        .add_torrent(AddTorrentRequest {
            name: "hook.mkv".to_owned(),
            total_bytes: 5,
            metainfo,
            save_path: String::new(),
        })
        .await
        .expect("add torrent");

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let status = connection::hook_status(&client, 10).await.expect("status");
        if status.runs.iter().any(|run| run.event == "added") {
            assert_eq!(status.pending, 0, "the event is acknowledged");
            assert!(status.runs[0].succeeded, "{:?}", status.runs[0]);
            assert_eq!(status.runs[0].torrent_name, "hook.mkv");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the hook did not run: {status:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
