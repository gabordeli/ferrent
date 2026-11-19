//! Connection helpers shared by the GUI shell and its tests.

use std::fs;
use std::path::PathBuf;

use ferrent_protocol::client::{AuthClient, connect as protocol_connect};
use ferrent_protocol::tls::fingerprint_from_pem;
use ferrent_protocol::v1::{
    GetStatusRequest, ListTorrentsRequest, SortField, Torrent,
};

use crate::config::ServerProfile;

/// Number of rows fetched per server-side page request.
pub const PAGE_SIZE: u32 = 100;

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

/// Server-side list query.
#[derive(Debug, Clone)]
pub struct ListQuery {
    pub filter: String,
    pub sort: SortField,
    pub descending: bool,
    pub offset: u32,
    pub limit: u32,
}

impl Default for ListQuery {
    fn default() -> Self {
        Self {
            filter: String::new(),
            sort: SortField::Id,
            descending: false,
            offset: 0,
            limit: PAGE_SIZE,
        }
    }
}

/// One server-side page.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub torrents: Vec<Torrent>,
    pub total: u32,
}

/// Aggregate status shown by the server manager.
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub torrent_count: u32,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub free_space_bytes: u64,
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

/// Fetches one filtered, sorted page from the daemon.
pub async fn list(client: &AuthClient, query: &ListQuery) -> Result<Page, String> {
    let mut client = client.clone();
    let response = client
        .list_torrents(ListTorrentsRequest {
            filter: query.filter.clone(),
            sort: query.sort as i32,
            descending: query.descending,
            offset: query.offset,
            limit: query.limit,
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    let response = response.into_inner();
    Ok(Page {
        torrents: response.torrents,
        total: response.total,
    })
}

/// Fetches aggregate status for the server manager.
pub async fn status(client: &AuthClient) -> Result<ServerStatus, String> {
    let mut client = client.clone();
    let response = client
        .get_status(GetStatusRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(ServerStatus {
        torrent_count: response.torrent_count,
        download_rate: response.download_rate,
        upload_rate: response.upload_rate,
        free_space_bytes: response.free_space_bytes,
    })
}

/// Connects and validates one profile; used by the server manager.
pub async fn probe(config: &ServerConfig) -> Result<ServerStatus, String> {
    let client = connect(config).await?;
    status(&client).await
}
