//! Human-readable and stable JSON output.

use std::io::Write;

use ferrent_protocol::v1::watch_response::Event;
use ferrent_protocol::v1::{Torrent, TorrentFile, TorrentState, WatchResponse};
use serde_json::{Value, json};

use crate::error::CliError;

/// Writes one line to stdout, ignoring a closed pipe: `ferrent-cli list | head`
/// must end quietly instead of panicking inside `println!` (a broken pipe used
/// to reach the issue tracker as a panic).
macro_rules! outln {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

pub fn torrent_json(torrent: &Torrent) -> Value {
    json!({
        "id": torrent.id,
        "infohash": torrent.infohash,
        "name": torrent.name,
        "save_path": torrent.save_path,
        "queue_position": torrent.queue_position,
        "pause_reasons": torrent.pause_reasons,
        "label": torrent.label,
        "state": state_label(torrent.state),
        "progress": torrent.progress,
        "total_bytes": torrent.total_bytes,
        "download_rate": torrent.download_rate,
        "upload_rate": torrent.upload_rate,
        "seeds": torrent.seeds,
        "peers": torrent.peers,
        "error": torrent.error,
    })
}

pub fn print_json(value: &Value) {
    outln!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialize JSON value")
    );
}

pub fn files_json(files: &[TorrentFile]) -> Value {
    json!(
        files
            .iter()
            .map(|file| json!({
                "path": file.path,
                "size": file.size,
                "progress": file.progress,
                "priority": file.priority,
            }))
            .collect::<Vec<_>>()
    )
}

pub fn print_files(files: &[TorrentFile]) {
    for file in files {
        outln!(
            "{:>3}  {:>12}  {:>6.1}%  {}",
            file.priority,
            format_bytes(file.size),
            file.progress * 100.0,
            file.path
        );
    }
}

pub fn print_torrents(torrents: &[Torrent]) {
    outln!(
        "{:>6}  {:<12}  {:>8}  {:>12}  {:>12}  NAME",
        "ID",
        "STATE",
        "PROGRESS",
        "DOWN",
        "UP"
    );
    for torrent in torrents {
        outln!(
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

/// Says which slice of the match set the table above shows.
pub fn print_page_note(first: u32, last: u32, total: u32) {
    outln!(
        "\n{}–{} of {} torrents (--offset/--limit to page)",
        first,
        last,
        total
    );
}

/// Dry-run listing for `import-deluge`: torrents first, then the plugin
/// configs the import would carry over.
pub fn print_import_table(report: &crate::import_deluge::Report) {
    outln!("{:>9}  {:<44}  {:<10}  INFOHASH", "SIZE", "NAME", "LABEL");
    for torrent in &report.torrents {
        outln!(
            "{:>9}  {:<44}  {:<10}  {}",
            format_bytes(torrent.total_bytes),
            short(&torrent.name, 44),
            torrent.label.as_deref().unwrap_or("—"),
            torrent.info_hash
        );
    }
    let labelled = report
        .torrents
        .iter()
        .filter(|torrent| torrent.label.is_some())
        .count();
    outln!("\n{} torrents, {labelled} labelled", report.torrents.len());

    let plugins = &report.plugins;
    outln!(
        "plugins: label {} label / {} assignment, autoadd {} watch dir, execute {} command, scheduler {}/{} red",
        plugins.labels.len(),
        plugins.assignments.len(),
        plugins.watch_dirs.len(),
        plugins.commands.len(),
        plugins.scheduler_red,
        plugins.scheduler_cells
    );
    for dir in &plugins.watch_dirs {
        outln!(
            "  autoadd {}{} -> {}",
            dir.path,
            if dir.enabled { "" } else { " (disabled)" },
            dir.download_location.as_deref().unwrap_or("(default)")
        );
    }
    for command in &plugins.commands {
        outln!("  execute {}: {}", command.event, command.command);
    }
    if !report.warnings.is_empty() {
        outln!("\nwarnings:");
        for warning in &report.warnings {
            outln!("  {warning}");
        }
    }
}

pub fn print_torrent(torrent: &Torrent) {
    outln!("id:       {}", torrent.id);
    outln!("name:     {}", torrent.name);
    outln!("state:    {}", state_label(torrent.state));
    outln!("progress: {:.1}%", torrent.progress * 100.0);
    outln!("size:     {}", format_bytes(torrent.total_bytes));
    outln!("save:     {}", torrent.save_path);
    outln!("down:     {}", format_rate(torrent.download_rate));
    outln!("up:       {}", format_rate(torrent.upload_rate));
    if !torrent.error.is_empty() {
        outln!("error:    {}", torrent.error);
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
            Event::ConfigChanged(change) => json!({
                "event": "config_changed",
                "revision": change.revision,
            }),
            Event::Finished(finished) => json!({
                "event": "finished",
                "id": finished.id,
            }),
        };
        outln!(
            "{}",
            serde_json::to_string(&value).expect("serialize watch event")
        );
    } else {
        match event {
            Event::Added(torrent) => outln!("added {} {}", torrent.id, torrent.name),
            Event::StateChanged(change) => {
                outln!("state {} {}", change.id, state_label(change.state));
            }
            Event::Progress(progress) => outln!(
                "progress {} {:.1}% down {} up {}",
                progress.id,
                progress.progress * 100.0,
                format_rate(progress.download_rate),
                format_rate(progress.upload_rate)
            ),
            Event::Removed(removed) => outln!("removed {}", removed.id),
            Event::ConfigChanged(change) => {
                outln!("config revision {}", change.revision);
            }
            Event::Finished(finished) => {
                outln!("finished {}", finished.id);
            }
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

fn short(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_owned();
    }
    let mut text: String = name.chars().take(max.saturating_sub(1)).collect();
    text.push('…');
    text
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

pub fn format_bytes(bytes: u64) -> String {
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
