//! Optional Prometheus endpoint (M8-T4).
//!
//! The daemon speaks gRPC only, so this is a one-path HTTP server: `GET
//! /metrics`, optionally behind a bearer token. It reads the state mirror and
//! `/proc/self`, nothing else.

use std::sync::Arc;

use ferrent_engine::TorrentState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::rpc::AppState;

/// Renders the exposition text. Pure, so it is testable without a socket.
#[must_use]
pub fn render(
    torrents: &[ferrent_engine::TorrentStatus],
    rss_bytes: u64,
    open_fds: usize,
) -> String {
    let count = |state: TorrentState| {
        torrents
            .iter()
            .filter(|torrent| torrent.state == state)
            .count()
    };
    let download: u64 = torrents.iter().map(|torrent| torrent.download_rate).sum();
    let upload: u64 = torrents.iter().map(|torrent| torrent.upload_rate).sum();
    let peers: u32 = torrents.iter().map(|torrent| torrent.peers).sum();

    let mut out = String::new();
    out.push_str("# HELP ferrent_torrents Torrents in the state mirror\n");
    out.push_str("# TYPE ferrent_torrents gauge\n");
    out.push_str(&format!("ferrent_torrents {}\n", torrents.len()));
    out.push_str("# TYPE ferrent_torrents_seeding gauge\n");
    out.push_str(&format!(
        "ferrent_torrents_seeding {}\n",
        count(TorrentState::Seeding)
    ));
    out.push_str("# TYPE ferrent_torrents_downloading gauge\n");
    out.push_str(&format!(
        "ferrent_torrents_downloading {}\n",
        count(TorrentState::Downloading)
    ));
    out.push_str("# TYPE ferrent_torrents_paused gauge\n");
    out.push_str(&format!(
        "ferrent_torrents_paused {}\n",
        count(TorrentState::Paused)
    ));
    out.push_str("# TYPE ferrent_download_rate_bytes_per_second gauge\n");
    out.push_str(&format!(
        "ferrent_download_rate_bytes_per_second {download}\n"
    ));
    out.push_str("# TYPE ferrent_upload_rate_bytes_per_second gauge\n");
    out.push_str(&format!("ferrent_upload_rate_bytes_per_second {upload}\n"));
    out.push_str("# TYPE ferrent_peers gauge\n");
    out.push_str(&format!("ferrent_peers {peers}\n"));
    out.push_str("# TYPE ferrent_process_resident_memory_bytes gauge\n");
    out.push_str(&format!(
        "ferrent_process_resident_memory_bytes {rss_bytes}\n"
    ));
    out.push_str("# TYPE ferrent_process_open_fds gauge\n");
    out.push_str(&format!("ferrent_process_open_fds {open_fds}\n"));
    out
}

fn rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map_or(0, |kb| kb * 1024)
}

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map_or(0, |entries| entries.count())
}

/// Serves `GET /metrics` until the process ends.
pub async fn serve(listen: String, token: Option<String>, state: Arc<AppState>) {
    let listener = match TcpListener::bind(&listen).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("metrics: cannot bind {listen}: {error}");
            return;
        }
    };
    println!("metrics listening on http://{listen}/metrics");
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        let token = token.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            let Ok(read) = socket.read(&mut buffer).await else {
                return;
            };
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let authorized = token.as_deref().is_none_or(|expected| {
                request.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, value)| {
                        name.eq_ignore_ascii_case("authorization")
                            && value
                                .trim()
                                .strip_prefix("Bearer ")
                                .is_some_and(|token| token == expected)
                    })
                })
            });
            let (status, body) = if !request.starts_with("GET /metrics ") {
                ("404 Not Found", "not found\n".to_owned())
            } else if !authorized {
                ("401 Unauthorized", "unauthorized\n".to_owned())
            } else {
                let body = render(&state.mirror.all(), rss_bytes(), open_fds());
                ("200 OK", body)
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: text/plain; version=0.0.4\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn torrent(id: u64, state: TorrentState, peers: u32) -> ferrent_engine::TorrentStatus {
        ferrent_engine::TorrentStatus {
            id,
            infohash: format!("{id:040x}"),
            name: format!("t{id}"),
            save_path: PathBuf::from("/data"),
            total_bytes: 1,
            state,
            progress: 0.5,
            download_rate: 100,
            upload_rate: 200,
            uploaded_bytes: 0,
            downloaded_bytes: 0,
            seeds: 1,
            peers,
            eta_seconds: 0,
            queue_position: -1,
        }
    }

    #[test]
    fn exports_counts_rates_and_process_metrics() {
        let body = render(
            &[
                torrent(1, TorrentState::Seeding, 2),
                torrent(2, TorrentState::Downloading, 3),
                torrent(3, TorrentState::Paused, 0),
            ],
            5 * 1024 * 1024,
            17,
        );
        assert!(body.contains("ferrent_torrents 3"));
        assert!(body.contains("ferrent_torrents_seeding 1"));
        assert!(body.contains("ferrent_torrents_downloading 1"));
        assert!(body.contains("ferrent_torrents_paused 1"));
        assert!(body.contains("ferrent_download_rate_bytes_per_second 300"));
        assert!(body.contains("ferrent_upload_rate_bytes_per_second 600"));
        assert!(body.contains("ferrent_peers 5"));
        assert!(body.contains("ferrent_process_resident_memory_bytes 5242880"));
        assert!(body.contains("ferrent_process_open_fds 17"));
        for line in body.lines().filter(|line| !line.starts_with('#')) {
            assert_eq!(line.split(' ').count(), 2, "malformed line: {line}");
        }
    }

    #[test]
    fn an_empty_daemon_still_exports_zeros() {
        let body = render(&[], 0, 0);
        assert!(body.contains("ferrent_torrents 0"));
        assert!(body.contains("ferrent_peers 0"));
    }
}
