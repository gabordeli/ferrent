//! Prepare a redb state directory with N real, seeded libtorrent torrents and
//! fastresume data for the M2-T4 restart/soak measurements.
//!
//! Run with:
//! `cargo run -p ferrent-daemon --features libtorrent --example mvp_prepare -- \
//!      --torrents 4000 --data-dir /tmp/ferrent-mvp-data`

#![cfg(feature = "libtorrent")]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrent_daemon::persist::{PersistedTorrent, save_torrents};
use ferrent_daemon::store::{RedbStore, StateStore};
use ferrent_engine::{
    AddTorrent, EngineSettings, LibtorrentEngine, TorrentEngine, TorrentSource, TorrentState,
    make_torrent,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut count = 4_000usize;
    let mut data_dir = PathBuf::from("/tmp/ferrent-mvp-data");
    let mut payload_bytes = 256usize;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--torrents" => count = args.next().expect("--torrents value").parse()?,
            "--data-dir" => data_dir = PathBuf::from(args.next().expect("--data-dir value")),
            "--payload-bytes" => {
                payload_bytes = args.next().expect("--payload-bytes value").parse()?;
            }
            "--help" | "-h" => {
                println!("usage: mvp_prepare [--torrents N] [--data-dir DIR] [--payload-bytes N]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    let _ = fs::remove_dir_all(&data_dir);
    let downloads = data_dir.join("downloads");
    fs::create_dir_all(&downloads)?;
    let db_path = data_dir.join("state.redb");
    let mut store = RedbStore::open(&db_path)?;

    let mut engine = LibtorrentEngine::new(EngineSettings {
        seed_mode: true,
        ..EngineSettings::default()
    });
    let mut torrents = Vec::with_capacity(count);
    let payload_template = vec![b'x'; payload_bytes];

    let add_start = Instant::now();
    for index in 0..count {
        let name = format!("payload-{index:05}.bin");
        let mut payload = payload_template.clone();
        let tag = format!("ferrent-{index:05}");
        payload[..tag.len()].copy_from_slice(tag.as_bytes());
        payload[payload_bytes - 1] = (index % 251) as u8;
        fs::write(downloads.join(&name), &payload)?;

        let metainfo = make_torrent(&name, &payload);
        let id = engine.add_torrent(AddTorrent {
            name: name.clone(),
            total_bytes: payload.len() as u64,
            source: TorrentSource::Metainfo(metainfo.clone()),
            save_path: downloads.clone(),
            resume_data: None,
        })?;
        torrents.push((id, name, metainfo));
    }
    println!(
        "added={count} add_secs={:.3}",
        add_start.elapsed().as_secs_f64()
    );

    let settle_deadline = Instant::now() + Duration::from_secs(600);
    let mut last_report = Instant::now();
    loop {
        let statuses = engine.list();
        let seeding = statuses
            .iter()
            .filter(|status| status.state == TorrentState::Seeding)
            .count();
        if seeding == count {
            break;
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            let checking = statuses
                .iter()
                .filter(|status| status.state == TorrentState::Checking)
                .count();
            println!("waiting for check: seeding={seeding} checking={checking} total={count}");
            last_report = Instant::now();
        }
        if Instant::now() >= settle_deadline {
            return Err(format!("only {seeding} of {count} torrents reached seeding").into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    for (id, _, _) in &torrents {
        engine.request_resume_data(*id)?;
    }

    let mut resume: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    let resume_deadline = Instant::now() + Duration::from_secs(60);
    loop {
        for (id, _, _) in &torrents {
            if resume.contains_key(id) {
                continue;
            }
            if let Some(data) = engine.take_resume_data(*id)? {
                resume.insert(*id, data);
            }
        }
        if resume.len() == torrents.len() {
            break;
        }
        if Instant::now() >= resume_deadline {
            for (id, _, _) in &torrents {
                if !resume.contains_key(id) {
                    let _ = engine.request_resume_data(*id);
                }
            }
            return Err(format!(
                "only {} of {} resume blobs arrived",
                resume.len(),
                torrents.len()
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let records: Vec<PersistedTorrent> = torrents
        .iter()
        .map(|(id, name, metainfo)| PersistedTorrent {
            id: *id,
            name: name.clone(),
            save_path: downloads.clone(),
            metainfo: metainfo.clone(),
            resume_data: resume.remove(id),
            paused_by_user: false,
        })
        .collect();
    save_torrents(&mut store, &records)?;
    store.flush()?;
    println!(
        "persisted={} fastresume={} data_dir={}",
        records.len(),
        records
            .iter()
            .filter(|torrent| torrent.resume_data.is_some())
            .count(),
        data_dir.display()
    );
    Ok(())
}
