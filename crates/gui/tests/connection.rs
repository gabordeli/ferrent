#![cfg(unix)]

mod support;

use std::time::Duration;

use ferrent_gui::connection::{ListQuery, ServerConfig, connect, list};
use support::{start_server, temp_ca};

#[tokio::test]
async fn reconnects_after_daemon_restart() {
    let tls = ferrent_daemon::rpc::TlsMaterial::generate().expect("generate TLS");
    let (dir, ca_path) = temp_ca(&tls);

    let first = start_server(0, tls.clone()).await;
    let port = first.addr.port();
    let config = ServerConfig {
        server: format!("https://localhost:{port}"),
        ca: ca_path,
        token: "secret".to_owned(),
    };

    let client = connect(&config).await.expect("connect");
    let page = list(&client, &ListQuery::default()).await.expect("list");
    assert!(page.torrents.is_empty());
    assert_eq!(page.total, 0);
    drop(client);

    first.shutdown();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        connect(&config).await.is_err(),
        "server should be down after shutdown"
    );

    let second = start_server(port, tls).await;
    let client = connect(&config).await.expect("reconnect");
    let page = list(&client, &ListQuery::default())
        .await
        .expect("list after restart");
    assert!(page.torrents.is_empty());
    second.shutdown();

    let _ = std::fs::remove_dir_all(dir);
}
