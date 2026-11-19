//! Connection helpers shared by the GUI shell and its tests.

use std::fs;
use std::path::PathBuf;

use ferrent_protocol::client::{AuthClient, connect as protocol_connect};
use ferrent_protocol::tls::fingerprint_from_pem;
use ferrent_protocol::v1::{ListTorrentsRequest, SortField, Torrent};

use crate::config::ServerProfile;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub server: String,
    pub ca: PathBuf,
    pub token: String,
}

impl From<&ServerProfile> for ServerConfig {
    fn from(profile: &ServerProfile) -> Self {
        Self {
            server: profile.server.clone(),
            ca: profile.ca.clone(),
            token: profile.token.clone(),
        }
    }
}

/// Reads the CA certificate and returns its SHA-256 fingerprint.
pub fn fingerprint(config: &ServerConfig) -> Result<String, String> {
    let pem = fs::read_to_string(&config.ca)
        .map_err(|error| format!("cannot read CA {}: {error}", config.ca.display()))?;
    fingerprint_from_pem(&pem)
        .map_err(|error| format!("invalid CA {}: {error}", config.ca.display()))
}

/// Connects over TLS using the CA certificate as the pinned trust anchor.
pub async fn connect(config: &ServerConfig) -> Result<AuthClient, String> {
    let pem = fs::read_to_string(&config.ca)
        .map_err(|error| format!("cannot read CA {}: {error}", config.ca.display()))?;
    protocol_connect(&config.server, &pem, &config.token)
        .await
        .map_err(|error| error.to_string())
}

/// Returns the first page of torrents from the daemon.
pub async fn list(client: &AuthClient) -> Result<Vec<Torrent>, String> {
    let mut client = client.clone();
    let response = client
        .list_torrents(ListTorrentsRequest {
            filter: String::new(),
            sort: SortField::Id as i32,
            descending: false,
            offset: 0,
            limit: 100,
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(response.into_inner().torrents)
}
