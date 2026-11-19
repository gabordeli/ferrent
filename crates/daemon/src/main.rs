//! `ferrentd`: TLS gRPC daemon with persisted torrent state.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrent_daemon::persist;
use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::{StateMirror, TorrentQuery};
use ferrent_daemon::store::{RedbStore, StateStore};
#[cfg(not(feature = "libtorrent"))]
use ferrent_engine::MockEngine;
use ferrent_engine::TorrentEngine;
#[cfg(feature = "libtorrent")]
use ferrent_engine::{EngineSettings, LibtorrentEngine};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let data_dir = env::var_os("FERRENT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./ferrent-data"));
    let token = env::var("FERRENT_TOKEN")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "FERRENT_TOKEN is required"))?;
    let bind = env::var("FERRENT_BIND").unwrap_or_else(|_| "127.0.0.1:50051".to_owned());

    let tls = TlsMaterial::load_or_generate(&data_dir)?;
    let store = RedbStore::open(data_dir.join("state.redb"))?;
    let persisted = persist::load_torrents(&store)?;
    let restored_count = persisted.len();

    #[cfg(feature = "libtorrent")]
    let mut engine: Box<dyn TorrentEngine> =
        Box::new(LibtorrentEngine::new(EngineSettings::default()));
    #[cfg(not(feature = "libtorrent"))]
    let mut engine: Box<dyn TorrentEngine> = Box::new(MockEngine::new());

    let events = engine.subscribe();
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(1024);
    let _mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());

    for torrent in persisted {
        let id = torrent.id;
        if let Err(error) = engine.restore_torrent(id, torrent.into_add_torrent()) {
            eprintln!("failed to restore torrent {id}: {error}");
        }
    }

    let store: Box<dyn StateStore> = Box::new(store);
    let state = Arc::new(AppState::new(engine, mirror, watch, store, token));

    let listener = TcpListener::bind(&bind).await?;
    let local = listener.local_addr()?;
    println!("ferrentd listening on https://{local}");
    println!("restored_torrents={restored_count}");
    println!("startup_ms={:.1}", started.elapsed().as_secs_f64() * 1000.0);
    println!("TLS fingerprint: {}", tls.fingerprint);
    println!(
        "CA certificate: {}",
        data_dir.join("tls-cert.pem").display()
    );

    tokio::select! {
        result = serve(listener, Arc::clone(&state), &tls) => result?,
        signal = shutdown_signal() => {
            signal?;
            println!("ferrentd shutting down; saving fastresume...");
            save_fastresumes(Arc::clone(&state)).await;
        }
    }

    Ok(())
}

async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn save_fastresumes(state: Arc<AppState>) {
    let ids: Vec<u64> = state
        .mirror_handle()
        .query(&TorrentQuery::default())
        .into_iter()
        .map(|torrent| torrent.id)
        .collect();

    let engine = state.engine_handle();
    let store = state.store_handle();
    let _ = tokio::task::spawn_blocking(move || {
        let mut engine = engine.lock().expect("engine mutex poisoned");
        for id in &ids {
            let _ = engine.request_resume_data(*id);
        }

        let mut remaining: BTreeSet<u64> = ids.iter().copied().collect();
        let mut entries = Vec::with_capacity(ids.len());
        let deadline = Instant::now() + Duration::from_secs(30);
        while !remaining.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            for id in remaining.clone() {
                if let Ok(Some(data)) = engine.take_resume_data(id) {
                    remaining.remove(&id);
                    entries.push((id, data));
                }
            }
        }
        if !remaining.is_empty() {
            eprintln!(
                "fastresume missing for {} of {} torrents",
                remaining.len(),
                ids.len()
            );
        }

        let mut store = store.lock().expect("state store mutex poisoned");
        if let Err(error) = persist::save_fastresumes(&mut **store, &entries) {
            eprintln!("failed to save fastresume: {error}");
        } else {
            println!(
                "saved fastresume for {} of {} torrents",
                entries.len(),
                ids.len()
            );
        }
    })
    .await;
}
