//! Human-readable and stable JSON output.

use std::io::Write;

use ferrent_protocol::v1::watch_response::Event;
use ferrent_protocol::v1::{Torrent, TorrentState, WatchResponse};
use serde_json::{Value, json};

use crate::error::CliError;

pub fn torrent_json(torrent: &Torrent) -> Value {
    json!({
        "id": torrent.id,
        "name": torrent.name,
        "state": state_label(torrent.state),
        "progress": torrent.progress,
        "total_bytes": torrent.total_bytes,
        "download_rate": torrent.download_rate,
        "upload_rate": torrent.upload_rate,
        "error": torrent.error,
    })
}

pub fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialize JSON value")
    );
}

pub fn print_torrents(torrents: &[Torrent]) {
    println!(
        "{:>6}  {:<12}  {:>8}  {:>12}  {:>12}  NAME",
        "ID", "STATE", "PROGRESS", "DOWN", "UP"
    );
    for torrent in torrents {
        println!(
            "{:>6}  {:<12}  {:>8}  {:>12}  {:>12}  {}",
            torrent.id,
            state_label(torrent.state),
            format!("{:.1}%", torrent.progress * 100.0),
            format_rate(torrent.download_rate),
            format_rate(torrent.upload_rate),
            torrent.name
        );
    }
}

pub fn print_torrent(torrent: &Torrent) {
    println!("id:       {}", torrent.id);
    println!("name:     {}", torrent.name);
    println!("state:    {}", state_label(torrent.state));
    println!("progress: {:.1}%", torrent.progress * 100.0);
    println!("size:     {}", format_bytes(torrent.total_bytes));
    println!("down:     {}", format_rate(torrent.download_rate));
    println!("up:       {}", format_rate(torrent.upload_rate));
    if !torrent.error.is_empty() {
        println!("error:    {}", torrent.error);
    }
}

pub fn print_event(event: &WatchResponse, json_mode: bool) -> Result<(), CliError> {
    let Some(event) = &event.event else {
        return Ok(());
    };

    if json_mode {
        let value = match event {
            Event::Added(torrent) => json!({
                "event": "added",
                "torrent": torrent_json(torrent),
            }),
            Event::StateChanged(change) => json!({
                "event": "state_changed",
                "id": change.id,
                "state": state_label(change.state),
            }),
            Event::Progress(progress) => json!({
                "event": "progress",
                "id": progress.id,
                "progress": progress.progress,
                "download_rate": progress.download_rate,
                "upload_rate": progress.upload_rate,
            }),
            Event::Removed(removed) => json!({
                "event": "removed",
                "id": removed.id,
            }),
        };
        println!(
            "{}",
            serde_json::to_string(&value).expect("serialize watch event")
        );
    } else {
        match event {
            Event::Added(torrent) => println!("added {} {}", torrent.id, torrent.name),
            Event::StateChanged(change) => {
                println!("state {} {}", change.id, state_label(change.state));
            }
            Event::Progress(progress) => println!(
                "progress {} {:.1}% down {} up {}",
                progress.id,
                progress.progress * 100.0,
                format_rate(progress.download_rate),
                format_rate(progress.upload_rate)
            ),
            Event::Removed(removed) => println!("removed {}", removed.id),
        }
    }
    std::io::stdout().flush()?;
    Ok(())
}

pub fn state_label(state: i32) -> String {
    let state = TorrentState::try_from(state).unwrap_or(TorrentState::Unspecified);
    state
        .as_str_name()
        .strip_prefix("TORRENT_STATE_")
        .unwrap_or("UNSPECIFIED")
        .to_ascii_lowercase()
}

fn format_rate(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B/s".to_owned();
    }
    const UNITS: [&str; 4] = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
