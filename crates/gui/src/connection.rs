//! Connection helpers shared by the GUI shell and its tests.

use std::fs;
use std::path::PathBuf;

use ferrent_protocol::client::{AuthClient, connect as protocol_connect};
use ferrent_protocol::tls::fingerprint_from_pem;
use ferrent_protocol::v1::{
    CreateLabelRequest, CreateTorrentRequest, DeleteLabelRequest, GetAutoAddStatusRequest,
    GetConfigRequest, GetDisksRequest, GetHookStatusRequest, GetNotificationStatusRequest,
    GetRssStatusRequest, GetStatusRequest, GetTorrentFilesRequest, GetTorrentPeersRequest,
    GetTorrentTrackersRequest, Label, LabelEntry, ListLabelsRequest, ListTorrentsRequest,
    RecheckTorrentRequest, RenameTorrentFileRequest, ScanAutoAddRequest,
    SendTestNotificationRequest, SetConfigRequest, SetFilePrioritiesRequest, SetSequentialRequest,
    SetSuperSeedingRequest, SetTorrentLabelRequest, SortField, Torrent, TorrentFile, TorrentPeer,
    TorrentTracker, UpdateLabelRequest,
};

use crate::config::ServerProfile;
use crate::settings::{DiskRow, ZoneView};

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
    pub state: Option<i32>,
    pub label: String,
    pub sort: SortField,
    pub descending: bool,
    pub offset: u32,
    pub limit: u32,
}

impl Default for ListQuery {
    fn default() -> Self {
        Self {
            filter: String::new(),
            state: None,
            label: String::new(),
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

/// Aggregate status shown by the server manager and the sidebar counters.
#[derive(Debug, Clone, Default)]
pub struct ServerStatus {
    pub torrent_count: u32,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub free_space_bytes: u64,
    pub downloading: u32,
    pub seeding: u32,
    pub queued: u32,
    pub paused: u32,
    pub error: u32,
    pub checking: u32,
    pub moving: u32,
    pub active: u32,
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
            state_filter: query.state.unwrap_or(0),
            label_filter: query.label.clone(),
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
        downloading: response.downloading_count,
        seeding: response.seeding_count,
        queued: response.queued_count,
        paused: response.paused_count,
        error: response.error_count,
        checking: response.checking_count,
        moving: response.moving_count,
        active: response.active_count,
    })
}

/// Connects and validates one profile; used by the server manager.
pub async fn probe(config: &ServerConfig) -> Result<ServerStatus, String> {
    let client = connect(config).await?;
    status(&client).await
}

/// Fetches the file list of one torrent.
pub async fn files(client: &AuthClient, id: u64) -> Result<Vec<TorrentFile>, String> {
    let mut client = client.clone();
    let response = client
        .get_torrent_files(GetTorrentFilesRequest { id })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.files)
}

/// Asks the daemon to re-check a torrent's data on disk.
pub async fn recheck_torrent(client: &AuthClient, id: u64) -> Result<(), String> {
    let mut client = client.clone();
    client
        .recheck_torrent(RecheckTorrentRequest { id })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(())
}

/// Renames one file inside a torrent and returns the refreshed file list.
pub async fn rename_torrent_file(
    client: &AuthClient,
    id: u64,
    index: u32,
    name: &str,
) -> Result<Vec<TorrentFile>, String> {
    let mut client = client.clone();
    let response = client
        .rename_torrent_file(RenameTorrentFileRequest {
            id,
            index,
            name: name.to_owned(),
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.files)
}

/// Downloads the torrent's pieces in order (or back to rarest-first).
pub async fn set_sequential(client: &AuthClient, id: u64, on: bool) -> Result<(), String> {
    let mut client = client.clone();
    client
        .set_sequential(SetSequentialRequest { id, on })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(())
}

/// Turns super-seeding on or off for one torrent.
pub async fn set_super_seeding(client: &AuthClient, id: u64, on: bool) -> Result<(), String> {
    let mut client = client.clone();
    client
        .set_super_seeding(SetSuperSeedingRequest { id, on })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?;
    Ok(())
}

/// Fetches the connected peers of one torrent.
pub async fn peers(client: &AuthClient, id: u64) -> Result<Vec<TorrentPeer>, String> {
    let mut client = client.clone();
    let response = client
        .get_torrent_peers(GetTorrentPeersRequest { id })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.peers)
}

/// Fetches the trackers of one torrent.
pub async fn trackers(client: &AuthClient, id: u64) -> Result<Vec<TorrentTracker>, String> {
    let mut client = client.clone();
    let response = client
        .get_torrent_trackers(GetTorrentTrackersRequest { id })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.trackers)
}

/// Replaces the per-file priorities and returns the refreshed file list.
pub async fn set_file_priorities(
    client: &AuthClient,
    id: u64,
    priorities: Vec<i32>,
) -> Result<Vec<TorrentFile>, String> {
    let mut client = client.clone();
    let response = client
        .set_file_priorities(SetFilePrioritiesRequest { id, priorities })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.files)
}

/// Per-feed state of the RSS watcher.
pub async fn rss_status(client: &AuthClient) -> Result<Vec<ferrent_protocol::v1::RssFeed>, String> {
    let mut client = client.clone();
    let response = client
        .get_rss_status(GetRssStatusRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.feeds)
}

/// Builds a .torrent for a path on the daemon host.
pub async fn create_torrent(
    client: &AuthClient,
    path: String,
) -> Result<(Vec<u8>, String), String> {
    let mut client = client.clone();
    let response = client
        .create_torrent(CreateTorrentRequest {
            path,
            piece_length: 0,
            trackers: Vec::new(),
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok((response.metainfo, response.infohash))
}

/// The daemon configuration document and its revision.
#[derive(Debug, Clone, Default)]
pub struct ConfigSnapshot {
    pub toml: String,
    pub revision: u64,
    pub env_locked: Vec<String>,
}

/// Fetches the configuration document.
pub async fn config(client: &AuthClient) -> Result<ConfigSnapshot, String> {
    let mut client = client.clone();
    let response = client
        .get_config(GetConfigRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(ConfigSnapshot {
        toml: response.toml,
        revision: response.revision,
        env_locked: response.env_locked,
    })
}

/// Replaces the configuration document; a stale revision is rejected.
pub async fn set_config(client: &AuthClient, toml: &str, revision: u64) -> Result<u64, String> {
    let mut client = client.clone();
    let response = client
        .set_config(SetConfigRequest {
            toml: toml.to_owned(),
            revision,
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.revision)
}

/// Lists the label definitions with their assignment counts.
pub async fn labels(client: &AuthClient) -> Result<Vec<LabelEntry>, String> {
    let mut client = client.clone();
    let response = client
        .list_labels(ListLabelsRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.labels)
}

/// Creates or updates one label definition.
pub async fn save_label(client: &AuthClient, label: Label, create: bool) -> Result<Label, String> {
    let mut client = client.clone();
    let response = if create {
        client
            .create_label(CreateLabelRequest { label: Some(label) })
            .await
    } else {
        client
            .update_label(UpdateLabelRequest { label: Some(label) })
            .await
    };
    Ok(response
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner()
        .label
        .unwrap_or_default())
}

/// Deletes a label and its assignments; returns the cleared assignment count.
pub async fn delete_label(client: &AuthClient, name: &str) -> Result<u32, String> {
    let mut client = client.clone();
    let response = client
        .delete_label(DeleteLabelRequest {
            name: name.to_owned(),
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.cleared_assignments)
}

/// Assigns a label to one torrent; an empty name clears it.
pub async fn assign_label(client: &AuthClient, id: u64, label: &str) -> Result<String, String> {
    let mut client = client.clone();
    let response = client
        .set_torrent_label(SetTorrentLabelRequest {
            id,
            label: label.to_owned(),
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.label)
}

/// Fetches the AutoAdd watch directories with their runtime counters.
pub async fn auto_add_status(
    client: &AuthClient,
) -> Result<Vec<crate::settings::AutoAddStatusRow>, String> {
    let mut client = client.clone();
    let response = client
        .get_auto_add_status(GetAutoAddStatusRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response
        .dirs
        .into_iter()
        .map(|dir| crate::settings::AutoAddStatusRow {
            path: dir.path,
            enabled: dir.enabled,
            label: dir.label,
            move_to: dir.move_to,
            pending: dir.pending,
            added: dir.added,
            duplicates: dir.duplicates,
            invalid: dir.invalid,
            deferred: dir.deferred,
            last_error: dir.last_error,
            last_scan_unix: dir.last_scan_unix,
        })
        .collect())
}

/// Totals of one manual AutoAdd scan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanTotals {
    pub added: u32,
    pub duplicates: u32,
    pub failed: u32,
    pub invalid: u32,
    pub deferred: u32,
}

/// Scans one watch directory, or all of them when `path` is empty.
pub async fn scan_auto_add(client: &AuthClient, path: &str) -> Result<ScanTotals, String> {
    let mut client = client.clone();
    let response = client
        .scan_auto_add(ScanAutoAddRequest {
            path: path.to_owned(),
        })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(ScanTotals {
        added: response.added,
        duplicates: response.duplicates,
        failed: response.failed,
        invalid: response.invalid,
        deferred: response.deferred,
    })
}

/// The hook journal depth and the recent runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookStatus {
    pub pending: u32,
    pub runs: Vec<crate::settings::HookRunRow>,
}

/// Fetches the hook status and the most recent runs.
pub async fn hook_status(client: &AuthClient, limit: u32) -> Result<HookStatus, String> {
    let mut client = client.clone();
    let response = client
        .get_hook_status(GetHookStatusRequest { limit })
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(HookStatus {
        pending: response.pending,
        runs: response
            .runs
            .into_iter()
            .map(|run| crate::settings::HookRunRow {
                event: run.event,
                torrent_name: run.torrent_name,
                command: run.command,
                succeeded: run.succeeded,
                exit_code: run.exit_code,
                timed_out: run.timed_out,
                duration_ms: run.duration_ms,
                stdout: run.stdout,
                stderr: run.stderr,
                at_unix: run.at_unix,
            })
            .collect(),
    })
}

/// Notification outbox counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotifyStatus {
    pub pending: u32,
    pub dead: u32,
    pub channels: u32,
}

/// Fetches the notification outbox counters.
pub async fn notification_status(client: &AuthClient) -> Result<NotifyStatus, String> {
    let mut client = client.clone();
    let response = client
        .get_notification_status(GetNotificationStatusRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(NotifyStatus {
        pending: response.pending,
        dead: response.dead,
        channels: response.channels,
    })
}

/// The storage payload, flattened for the settings table.
pub async fn disks(client: &AuthClient) -> Result<Vec<DiskRow>, String> {
    let mut client = client.clone();
    let response = client
        .get_disks(GetDisksRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response
        .disks
        .into_iter()
        .map(|disk| DiskRow {
            mount: disk.mount,
            fs: disk.fs,
            free: disk.free_bytes,
            total: disk.total_bytes,
            torrents: disk.torrents,
            low: disk.low,
            zones: disk
                .zones
                .into_iter()
                .map(|zone| ZoneView {
                    path: zone.path,
                    label: zone.label,
                    staging: zone.staging,
                    enabled: zone.enabled,
                })
                .collect(),
        })
        .collect())
}

/// Queues a test delivery to every enabled webhook.
pub async fn send_test_notification(client: &AuthClient) -> Result<u32, String> {
    let mut client = client.clone();
    let response = client
        .send_test_notification(SendTestNotificationRequest {})
        .await
        .map_err(|error| format!("{:?}: {}", error.code(), error.message()))?
        .into_inner();
    Ok(response.queued)
}
