//! Torrent mutations and `.torrent` uploads used by the GUI.

use std::path::Path;

use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, PauseTorrentRequest, RemoveTorrentRequest, ResumeTorrentRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentAction {
    Pause,
    Resume,
    Remove,
}

impl TorrentAction {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Remove => "remove",
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
            TorrentAction::Remove => client
                .remove_torrent(RemoveTorrentRequest { id: *id })
                .await
                .map(|_| ()),
        };
        if let Err(status) = result {
            errors.push(format!("{id}: {:?}: {}", status.code(), status.message()));
        }
    }
    errors
}

/// Reads a local `.torrent` and sends its raw metainfo bytes to the daemon.
pub async fn add(client: &AuthClient, path: &Path, save_path: &str) -> Result<u64, String> {
    let metainfo =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("torrent")
        .to_owned();
    let mut client = client.clone();
    let response = client
        .add_torrent(AddTorrentRequest {
            name,
            metainfo,
            save_path: save_path.to_owned(),
            total_bytes: 0,
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(response.into_inner().id)
}
