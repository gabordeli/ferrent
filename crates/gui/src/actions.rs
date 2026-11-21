//! Torrent mutations and `.torrent` uploads used by the GUI.

use std::path::Path;

use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, MoveTorrentRequest, PauseTorrentRequest, QueueDirection,
    RemoveTorrentRequest, ResumeTorrentRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentAction {
    Pause,
    Resume,
    Remove { delete_files: bool },
}

impl TorrentAction {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Remove { .. } => "remove",
        }
    }
}

/// Applies one action to every id, returning the errors in id order.
pub async fn apply(client: &AuthClient, action: TorrentAction, ids: &[u64]) -> Vec<String> {
    let mut errors = Vec::new();
    for id in ids {
        let mut client = client.clone();
        let result = match action {
            TorrentAction::Pause => client
                .pause_torrent(PauseTorrentRequest { id: *id })
                .await
                .map(|_| ()),
            TorrentAction::Resume => client
                .resume_torrent(ResumeTorrentRequest { id: *id })
                .await
                .map(|_| ()),
            TorrentAction::Remove { delete_files } => client
                .remove_torrent(RemoveTorrentRequest {
                    id: *id,
                    delete_files,
                })
                .await
                .map(|_| ()),
        };
        if let Err(status) = result {
            errors.push(format!("{id}: {:?}: {}", status.code(), status.message()));
        }
    }
    errors
}

/// Moves one torrent up or down in the daemon's download queue.
pub async fn move_in_queue(client: &AuthClient, id: u64, up: bool) -> Result<(), String> {
    let mut client = client.clone();
    let direction = if up {
        QueueDirection::Up
    } else {
        QueueDirection::Down
    };
    client
        .move_torrent(MoveTorrentRequest {
            id,
            direction: direction as i32,
        })
        .await
        .map(|_| ())
        .map_err(|status| format!("{id}: {:?}: {}", status.code(), status.message()))
}

/// Reads a local `.torrent` and sends its raw metainfo bytes to the daemon.
pub async fn add(
    client: &AuthClient,
    path: &Path,
    save_path: &str,
    display_name: &str,
    label: &str,
) -> Result<u64, String> {
    let metainfo =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let name = if display_name.trim().is_empty() {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("torrent")
            .to_owned()
    } else {
        display_name.trim().to_owned()
    };
    let mut client = client.clone();
    let response = client
        .add_torrent(AddTorrentRequest {
            name,
            metainfo,
            save_path: save_path.to_owned(),
            total_bytes: 0,
            label: label.to_owned(),
            magnet: String::new(),
            sequential: false,
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(response.into_inner().id)
}
