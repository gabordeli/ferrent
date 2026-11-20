//! `ferrentd`: TLS gRPC daemon with persisted torrent state.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrent_daemon::config::{ConfigStore, ResolvedConfig, read_token_file};
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::persist;
use ferrent_daemon::rpc::{AppState, TlsMaterial, serve, spawn_mirror};
use ferrent_daemon::state::{StateMirror, TorrentQuery};
use ferrent_daemon::store::{RedbStore, StateStore};
#[cfg(feature = "libtorrent")]
use ferrent_engine::LibtorrentEngine;
#[cfg(not(feature = "libtorrent"))]
use ferrent_engine::MockEngine;
use ferrent_engine::{EngineSettings, TorrentEngine};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _telemetry = ferrent_telemetry::init("ferrentd");
    let result = run().await;
    if let Err(error) = &result {
        ferrent_telemetry::capture_error(error.as_ref());
    }
    result
}

struct Config {
    store: ConfigStore,
    resolved: ResolvedConfig,
    token: String,
}

fn load_config() -> Result<Config, Box<dyn Error>> {
    let path = env::var_os("FERRENT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/ferrent/config.toml"));
    let store = ConfigStore::load(path)?;
    let resolved = store.resolved()?;
    let token = if let Ok(token) = env::var("FERRENT_TOKEN") {
        token
    } else {
        read_token_file(&resolved.token_file).map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "cannot read token file {}: {error}",
                    resolved.token_file.display()
                ),
            )
        })?
    };
    if token.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty API token").into());
    }
    Ok(Config {
        store,
        resolved,
        token,
    })
}

async fn run() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let Config {
        store: config_store,
        resolved,
        token,
    } = load_config()?;
    let data_dir = resolved.data_dir;
    let download_dir = resolved.download_dir;
    let bind = resolved.bind;
    let settings = EngineSettings {
        download_rate_limit: resolved.session.download_rate_limit,
        upload_rate_limit: resolved.session.upload_rate_limit,
        seed_mode: resolved.session.seed_mode,
        active_downloads: resolved.session.active_downloads,
        active_seeds: resolved.session.active_seeds,
        active_limit: resolved.session.active_limit,
    };

    let tls = TlsMaterial::load_or_generate(&data_dir)?;
    let store = RedbStore::open(data_dir.join("state.redb"))?;
    let mut persisted = persist::load_torrents(&store)?;
    let queue_order = persist::load_queue_order(&store)?;
    order_for_restore(&mut persisted, &queue_order);
    let restored_count = persisted.len();

    #[cfg(feature = "libtorrent")]
    let mut engine: Box<dyn TorrentEngine> = Box::new(LibtorrentEngine::new(settings));
    #[cfg(not(feature = "libtorrent"))]
    let mut engine: Box<dyn TorrentEngine> = {
        let mut engine = MockEngine::new();
        engine.set_settings(settings);
        Box::new(engine)
    };

    let events = engine.subscribe();
    let mirror = Arc::new(StateMirror::new());
    let (watch, _) = broadcast::channel(1024);
    let _mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());

    let mut control = TorrentControl::default();
    for torrent in persisted {
        let id = torrent.id;
        let paused_by_user = torrent.paused_by_user;
        if let Err(error) = engine.restore_torrent(id, torrent.into_add_torrent()) {
            eprintln!("failed to restore torrent {id}: {error}");
            continue;
        }
        if paused_by_user {
            control.set_user(id, true);
            if let Err(error) = engine.pause(id) {
                eprintln!("failed to pause restored torrent {id}: {error}");
            }
        }
    }

    let store: Box<dyn StateStore> = Box::new(store);
    let state = Arc::new(AppState::new(
        engine,
        mirror,
        watch,
        store,
        config_store,
        control,
        token,
        data_dir.clone(),
        download_dir.clone(),
    ));

    let listener = TcpListener::bind(&bind).await?;
    let local = listener.local_addr()?;
    println!("ferrentd listening on https://{local}");
    println!("restored_torrents={restored_count}");
    println!("download_dir={}", download_dir.display());
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
        .query(&TorrentQuery {
            sort: ferrent_daemon::state::TorrentSort::Queue,
            ..TorrentQuery::default()
        })
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
        if let Err(error) = persist::save_queue_order(&mut **store, &ids) {
            eprintln!("failed to save queue order: {error}");
        }
    })
    .await;
}

/// Restores torrents in the persisted queue order, so libtorrent reassigns the
/// same positions; ids missing from the record keep their id order at the end.
fn order_for_restore(torrents: &mut [persist::PersistedTorrent], order: &[u64]) {
    let rank: std::collections::BTreeMap<u64, usize> = order
        .iter()
        .enumerate()
        .map(|(rank, id)| (*id, rank))
        .collect();
    torrents.sort_by_key(|torrent| {
        (
            rank.get(&torrent.id).copied().unwrap_or(usize::MAX),
            torrent.id,
        )
    });
}
