//! `ferrentd`: TLS gRPC daemon.

use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
#[cfg(not(feature = "libtorrent"))]
use ferrent_engine::MockEngine;
use ferrent_engine::TorrentEngine;
#[cfg(feature = "libtorrent")]
use ferrent_engine::{EngineSettings, LibtorrentEngine};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let data_dir = env::var_os("FERRENT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./ferrent-data"));
    let token = env::var("FERRENT_TOKEN")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "FERRENT_TOKEN is required"))?;
    let bind = env::var("FERRENT_BIND").unwrap_or_else(|_| "127.0.0.1:50051".to_owned());

    let tls = TlsMaterial::load_or_generate(&data_dir)?;

    #[cfg(feature = "libtorrent")]
    let engine: Box<dyn TorrentEngine> = Box::new(LibtorrentEngine::new(EngineSettings::default()));
    #[cfg(not(feature = "libtorrent"))]
    let engine: Box<dyn TorrentEngine> = Box::new(MockEngine::new());

    let events = engine.subscribe();
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(1024);
    let _mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let state = Arc::new(AppState::new(engine, mirror, watch, token));

    let listener = TcpListener::bind(&bind).await?;
    let local = listener.local_addr()?;
    println!("ferrentd listening on https://{local}");
    println!("TLS fingerprint: {}", tls.fingerprint);
    println!(
        "CA certificate: {}",
        data_dir.join("tls-cert.pem").display()
    );

    serve(listener, state, &tls).await?;
    Ok(())
}
