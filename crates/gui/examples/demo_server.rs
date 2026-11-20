//! Measurement harness: an in-process daemon with N mock torrents, used to
//! exercise the GUI table at 4 000 rows.
//!
//! Run with:
//! `cargo run -p ferrent-gui --example demo_server -- --torrents 4000`

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::control::TorrentControl;
use ferrent_daemon::labels::LabelState;
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
    // `--zone PATH:LABEL` adds a library zone, `--staging PATH` a staging
    // zone; without them the demo builds its own folders under the data dir.
    let mut configured: Vec<(PathBuf, Option<String>)> = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--torrents" => torrents = args.next().expect("--torrents value").parse()?,
            "--port" => port = args.next().expect("--port value").parse()?,
            "--data-dir" => data_dir = PathBuf::from(args.next().expect("--data-dir value")),
            "--zone" => {
                let spec = args.next().expect("--zone value");
                let (path, label) = spec
                    .rsplit_once(':')
                    .map_or((spec.as_str(), "movies"), |(path, label)| (path, label));
                configured.push((PathBuf::from(path), Some(label.to_owned())));
            }
            "--staging" => {
                configured.push((PathBuf::from(args.next().expect("--staging value")), None))
            }
            "--help" | "-h" => {
                println!(
                    "usage: demo_server [--torrents N] [--port PORT] [--data-dir DIR] \
                     [--zone PATH:LABEL]... [--staging PATH]..."
                );
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
            4 => TorrentState::Moving,
            _ => TorrentState::Downloading,
        };
    }

    let mirror = Arc::new(StateMirror::new());
    mirror.seed(statuses);
    let events = engine.subscribe();
    let (watch, _) = broadcast::channel(256);
    let mirror_task = spawn_mirror(Arc::clone(&mirror), events, watch.clone());
    let mut labels = LabelState::default();
    for (name, color) in [
        ("movies", "#2f81f7"),
        ("series", "#a17bf0"),
        ("music", "#21c55d"),
        ("archive", "#f2cc15"),
    ] {
        labels
            .upsert(ferrent_daemon::labels::Label {
                name: name.to_owned(),
                color: color.to_owned(),
                download_rate_limit: 0,
                upload_rate_limit: 0,
                ..ferrent_daemon::labels::Label::default()
            })
            .expect("demo label");
    }
    for index in 0..torrents.min(40) {
        let name = match index % 4 {
            0 => "movies",
            1 => "series",
            2 => "music",
            _ => "archive",
        };
        labels
            .assign(index as u64, Some(name.to_owned()))
            .expect("assign");
    }
    let watch_dir = data_dir.join("watch");
    std::fs::create_dir_all(&watch_dir)?;
    std::fs::write(
        watch_dir.join("demo-auto.torrent"),
        b"d4:infod6:lengthi9e4:name8:demo.mkvee",
    )?;
    // Storage zones: configured ones or one library folder per label plus a
    // staging folder under the data dir.
    if configured.is_empty() {
        let media = data_dir.join("media");
        let staging = data_dir.join("progress");
        for (label, folder) in [
            ("movies", "Filmek"),
            ("series", "Sorozatok"),
            ("music", "Zene"),
            ("archive", "Archiv"),
        ] {
            let path = media.join(folder);
            std::fs::create_dir_all(&path)?;
            configured.push((path, Some(label.to_owned())));
        }
        std::fs::create_dir_all(&staging)?;
        configured.push((staging, None));
    }
    let mut zones = String::new();
    for (path, label) in &configured {
        match label {
            Some(label) => zones.push_str(&format!(
                "[[diskguard.zone]]\npath = {:?}\nlabel = \"{label}\"\n\n",
                path.display().to_string()
            )),
            None => zones.push_str(&format!(
                "[[diskguard.zone]]\npath = {:?}\nstaging = true\n\n",
                path.display().to_string()
            )),
        }
    }
    let config = ConfigStore::default();
    config.apply_toml(
        &format!(
            "[[autoadd]]\npath = {:?}\nlabel = \"movies\"\n\n[[hook]]\nevent = \"added\"\ncommand = \"/bin/sh\"\nargs = [\"-c\", \"true\"]\n\n[diskguard]\nenabled = true\nmin_free_bytes = 2147483648\nmargin_bytes = 1073741824\npoll_secs = 30\n\n{zones}",
            watch_dir.display().to_string()
        ),
        0,
    )?;
    let state = Arc::new(AppState::new(
        Box::new(engine),
        mirror,
        watch,
        Box::new(MemoryStore::new()),
        config,
        TorrentControl::default(),
        labels,
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
