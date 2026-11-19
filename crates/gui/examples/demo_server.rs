//! Measurement harness: an in-process daemon with N mock torrents, used to
//! exercise the GUI table at 4 000 rows.
//!
//! Run with:
//! `cargo run -p ferrent-gui --example demo_server -- --torrents 4000`

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::StateMirror;
use ferrent_daemon::store::MemoryStore;
use ferrent_engine::{
    AddTorrent, MockEngine, TorrentEngine, TorrentSource, TorrentState, TorrentStatus,
};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut torrents = 4_000usize;
    let mut port = 50_129u16;
    let mut data_dir = PathBuf::from("/tmp/ferrent-demo-data");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--torrents" => torrents = args.next().expect("--torrents value").parse()?,
            "--port" => port = args.next().expect("--port value").parse()?,
            "--data-dir" => data_dir = PathBuf::from(args.next().expect("--data-dir value")),
            "--help" | "-h" => {
                println!("usage: demo_server [--torrents N] [--port PORT] [--data-dir DIR]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let tls = TlsMaterial::load_or_generate(&data_dir)?;
    let mut engine = MockEngine::new();
    for index in 0..torrents {
        engine.add_torrent(AddTorrent {
            name: format!("torrent-{index:04}.mkv"),
            total_bytes: 4 * 1024 * 1024 * 1024,
            source: TorrentSource::Magnet(format!("magnet:?xt=urn:btih:{index:040}")),
            save_path: PathBuf::from("/tmp/ferrent-downloads"),
            resume_data: None,
        })?;
    }
    let mut statuses: Vec<TorrentStatus> = engine.list();
    for (index, status) in statuses.iter_mut().enumerate() {
        status.progress = ((index * 37) % 100) as f32 / 100.0;
        status.download_rate = (index as u64 * 131_071) % 5_000_000;
        status.upload_rate = (index as u64 * 97_003) % 900_000;
        status.state = match index % 7 {
            0 => TorrentState::Paused,
            1 => TorrentState::Seeding,
            2 => TorrentState::Checking,
            3 => TorrentState::Error("demo error".to_owned()),
            _ => TorrentState::Downloading,
        };
    }

    let mirror = Arc::new(StateMirror::new());
    mirror.seed(statuses);
    let events = engine.subscribe();
    let (watch, _) = broadcast::channel(256);
    let mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let state = Arc::new(AppState::new(
        Box::new(engine),
        mirror,
        watch,
        Box::new(MemoryStore::new()),
        "secret",
        data_dir.clone(),
    ));

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    println!("demo server: https://localhost:{}", addr.port());
    println!("CA: {}", data_dir.join("tls-cert.pem").display());
    println!("TLS fingerprint: {}", tls.fingerprint);
    println!("torrents: {torrents}");
    tokio::select! {
        result = serve(listener, state, &tls) => result?,
        _ = tokio::signal::ctrl_c() => {},
    }
    mirror_task.abort();
    Ok(())
}
