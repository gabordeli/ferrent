//! Typed daemon configuration: file schema, layering and validation.
//!
//! The file is `/etc/ferrent/config.toml` (or `$FERRENT_CONFIG`). The schema
//! keeps the original flat keys (`data_dir`, `download_dir`, `bind`,
//! `token_file`) and adds sections for the features that follow. Unknown keys
//! are rejected instead of silently ignored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ConfigError;
use crate::autoadd::WatchDir;
use crate::diskguard::Zone;
use crate::hooks::HookSpec;
use crate::notify::{TelegramRule, WebhookRule};
use crate::scheduler::Grid;

const DEFAULT_DATA_DIR: &str = "./ferrent-data";
const DEFAULT_BIND: &str = "127.0.0.1:50051";
const DEFAULT_TOKEN_FILE: &str = "/etc/ferrent/token";

/// The on-disk configuration file.
///
/// Every field has a default so a missing file (or a partial file) is valid.
/// `deny_unknown_fields` makes a typo a startup error rather than a silently
/// ignored setting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub data_dir: Option<PathBuf>,
    pub download_dir: Option<PathBuf>,
    pub bind: Option<String>,
    pub token_file: Option<PathBuf>,
    pub session: SessionConfig,
    pub queue: QueueConfig,
    /// Watch directories fed into the engine (M5-T1).
    pub autoadd: Vec<WatchDir>,
    /// Execute hooks (M5-T2).
    pub hook: Vec<HookSpec>,
    /// 24x7 scheduler (M5-T3).
    pub scheduler: SchedulerConfig,
    /// Notification channels (M5-T4).
    pub notify: NotifyConfig,
    /// Storage zones and disk guard thresholds (M6).
    pub diskguard: DiskGuardConfig,
    /// Ratio and seed-time policy (M8-T3).
    pub ratio: RatioConfig,
    /// Interface binding and link watching (M8-T4).
    pub network: NetworkConfig,
    /// Optional Prometheus endpoint (M8-T4).
    pub metrics: MetricsConfig,
    /// Client-API emulations (M8-T2).
    pub compat: CompatConfig,
    /// RSS feeds that add torrents on their own (M10-T4).
    pub rss: RssConfig,
}

/// Interface binding (M8-T4).
///
/// Empty values keep libtorrent's defaults, so an untouched config behaves
/// exactly like before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// libtorrent `listen_interfaces`, e.g. `0.0.0.0:6881` or `wg0:6881`.
    pub listen_interfaces: String,
    /// libtorrent `outgoing_interfaces`, e.g. `wg0` or `wg0/10.8.0.2`.
    pub outgoing_interfaces: String,
    /// Interface whose loss pauses every torrent, e.g. `wg0`.
    pub watch_interface: String,
    /// How long the interface must be back before torrents resume.
    pub grace_secs: u64,
    /// Outbound proxy: `none`, `http`, `http_pw`, `socks5` or `socks5_pw`.
    pub proxy_type: String,
    /// Proxy hostname or IP. Required for every type but `none`.
    pub proxy_host: String,
    /// Proxy port; `0` with a proxy type is an error.
    pub proxy_port: u16,
    pub proxy_username: String,
    pub proxy_password: String,
    /// Send peer connections through the proxy as well, not just trackers.
    pub proxy_peer_connections: bool,
    /// Peer encryption policy: `enabled`, `forced` or `disabled`.
    pub encryption: String,
    /// Ask the router to open the listen port.
    pub upnp: bool,
    /// Same through NAT-PMP.
    pub natpmp: bool,
    /// IP blocklist sources: local paths or http(s) URLs (eMule/PeerGuardian
    /// text or CIDR lines).
    pub blocklist: Vec<String>,
    /// Hours between automatic blocklist reloads; `0` disables the timer.
    pub blocklist_refresh_hours: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_interfaces: String::new(),
            outgoing_interfaces: String::new(),
            watch_interface: String::new(),
            grace_secs: 60,
            proxy_type: String::new(),
            proxy_host: String::new(),
            proxy_port: 0,
            proxy_username: String::new(),
            proxy_password: String::new(),
            proxy_peer_connections: false,
            encryption: String::new(),
            upnp: true,
            natpmp: true,
            blocklist: Vec::new(),
            blocklist_refresh_hours: 24,
        }
    }
}

/// The proxy types libtorrent accepts, with the spelling the config uses.
pub const PROXY_TYPES: [&str; 5] = ["http", "http_pw", "socks5", "socks5_pw", "none"];
/// Encryption policies accepted by the config.
pub const ENCRYPTION_POLICIES: [&str; 3] = ["enabled", "forced", "disabled"];

impl NetworkConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.grace_secs > 3_600 {
            return Err("network.grace_secs must be at most 3600".to_owned());
        }
        if !self.watch_interface.is_empty() && self.watch_interface.contains(['/', ':', ' ']) {
            return Err("network.watch_interface must be a plain interface name".to_owned());
        }
        let proxy = self.proxy_type.trim();
        if !proxy.is_empty() && !PROXY_TYPES.contains(&proxy) {
            return Err(format!(
                "network.proxy_type must be one of {}",
                PROXY_TYPES.join(", ")
            ));
        }
        if !proxy.is_empty() && proxy != "none" {
            if self.proxy_host.trim().is_empty() {
                return Err("network.proxy_host is required for a proxy".to_owned());
            }
            if self.proxy_port == 0 {
                return Err("network.proxy_port is required for a proxy".to_owned());
            }
        }
        let encryption = self.encryption.trim();
        if !encryption.is_empty() && !ENCRYPTION_POLICIES.contains(&encryption) {
            return Err(format!(
                "network.encryption must be one of {}",
                ENCRYPTION_POLICIES.join(", ")
            ));
        }
        Ok(())
    }
}

/// Client-API emulations. Off by default; each one binds its own port.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CompatConfig {
    pub qbittorrent: QbittorrentConfig,
}

/// qBittorrent Web API v2 emulation for Sonarr/Radarr (M8-T2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QbittorrentConfig {
    pub enabled: bool,
    pub listen: String,
    /// Empty username means "no login required" (loopback only, please).
    pub username: String,
    pub password: String,
    /// Category name → save path, exposed through `torrents/categories`.
    pub category_paths: std::collections::BTreeMap<String, String>,
}

impl Default for QbittorrentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:8090".to_owned(),
            username: "admin".to_owned(),
            password: String::new(),
            category_paths: std::collections::BTreeMap::new(),
        }
    }
}

impl QbittorrentConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && !self.listen.contains(':') {
            return Err("compat.qbittorrent.listen must be host:port".to_owned());
        }
        if self.enabled && self.username.is_empty() && self.password.is_empty() {
            eprintln!("warning: compat.qbittorrent has no credentials; keep it on loopback");
        }
        Ok(())
    }
}

/// Prometheus endpoint (M8-T4). Off by default; a token makes it
/// scrapeable from outside loopback without exposing it to the LAN.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub listen: String,
    /// When set, the scrape needs `Authorization: Bearer <token>`.
    pub token: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:9898".to_owned(),
            token: String::new(),
        }
    }
}

impl MetricsConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && !self.listen.contains(':') {
            return Err("metrics.listen must be host:port".to_owned());
        }
        Ok(())
    }
}

/// Ratio / seed-time auto-remove (M8-T3).
///
/// Zero means "off": a torrent is only acted on when a threshold is set,
/// either in its label override or in the session defaults here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RatioConfig {
    pub enabled: bool,
    /// Stop seeding when uploaded/downloaded reaches this (0 = off).
    pub stop_at_ratio: f32,
    /// Remove when uploaded/downloaded reaches this (0 = off).
    pub remove_at_ratio: f32,
    /// Remove after this many hours of seeding (0 = off).
    pub seed_time_hours: u32,
    /// Removal deletes the payload; without it only the torrent goes.
    pub remove_with_data: bool,
    /// Per-label overrides, matched by label name.
    pub label: Vec<LabelRatio>,
    /// Report the candidates without pausing or removing anything.
    pub dry_run: bool,
    /// Evaluation period in seconds.
    pub poll_secs: u64,
}

impl Default for RatioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stop_at_ratio: 0.0,
            remove_at_ratio: 0.0,
            seed_time_hours: 0,
            remove_with_data: false,
            label: Vec::new(),
            dry_run: false,
            poll_secs: 300,
        }
    }
}

impl RatioConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_secs < 30 || self.poll_secs > 3_600 {
            return Err("ratio.poll_secs must be between 30 and 3600".to_owned());
        }
        for (name, value) in [
            ("stop_at_ratio", self.stop_at_ratio),
            ("remove_at_ratio", self.remove_at_ratio),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("ratio.{name} must be a non-negative number"));
            }
        }
        for (index, label) in self.label.iter().enumerate() {
            if label.name.trim().is_empty() {
                return Err(format!("ratio.label[{index}].name must not be empty"));
            }
            if !label.stop_at_ratio.is_finite() || label.stop_at_ratio < 0.0 {
                return Err(format!(
                    "ratio.label[{index}].stop_at_ratio must be a non-negative number"
                ));
            }
            if !label.remove_at_ratio.is_finite() || label.remove_at_ratio < 0.0 {
                return Err(format!(
                    "ratio.label[{index}].remove_at_ratio must be a non-negative number"
                ));
            }
        }
        Ok(())
    }
}

/// A label's ratio overrides; zero fields inherit the session default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LabelRatio {
    pub name: String,
    pub stop_at_ratio: f32,
    pub remove_at_ratio: f32,
    pub seed_time_hours: u32,
    pub remove_with_data: bool,
}

/// Outbound notification settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NotifyConfig {
    pub enabled: bool,
    /// Webhook channels; the router fans out one delivery per matching rule.
    pub webhook: Vec<WebhookRule>,
    /// Telegram channels.
    pub telegram: Vec<TelegramRule>,
    /// Digest threshold for Telegram (0 = immediate).
    pub digest_events: u32,
    /// Digest window in seconds (0 = only the threshold flushes).
    pub digest_window_secs: u64,
    /// Quiet hours start (`HH:MM`); empty disables them.
    pub quiet_start: String,
    /// Quiet hours end (`HH:MM`).
    pub quiet_end: String,
    /// Timezone of the quiet hours.
    pub timezone: String,
    /// Only deliver events with this label (empty = all).
    pub label_filter: String,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            webhook: Vec::new(),
            telegram: Vec::new(),
            digest_events: 10,
            digest_window_secs: 30,
            quiet_start: String::new(),
            quiet_end: String::new(),
            timezone: "UTC".to_owned(),
            label_filter: String::new(),
        }
    }
}

impl NotifyConfig {
    /// Checks every configured webhook.
    pub fn validate(&self) -> Result<(), String> {
        for (index, rule) in self.webhook.iter().enumerate() {
            rule.validate()
                .map_err(|message| format!("notify.webhook[{index}]: {message}"))?;
        }
        for (index, rule) in self.telegram.iter().enumerate() {
            rule.validate()
                .map_err(|message| format!("notify.telegram[{index}]: {message}"))?;
        }
        if self.timezone.trim().parse::<chrono_tz::Tz>().is_err() {
            return Err(format!(
                "unknown notification timezone {:?}",
                self.timezone.trim()
            ));
        }
        crate::notify::parse_quiet(&self.quiet_start, &self.quiet_end)?;
        if self.digest_events > 1_000 {
            return Err("notify.digest_events must be at most 1000".to_owned());
        }
        if self.digest_window_secs > 86_400 {
            return Err("notify.digest_window_secs must be at most 86400".to_owned());
        }
        Ok(())
    }
}

/// What an add does when the staging filesystem is below the floor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LowAction {
    /// Refuse the add.
    #[default]
    Reject,
    /// Allow the add and journal a warning.
    Notify,
}

/// Storage zones and the disk guard thresholds.
///
/// Zones are the source of truth for where a label's finished payload lives
/// and where downloads of that label are staged; the guard part only reads
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DiskGuardConfig {
    /// Runs the guard loop (thresholds, pause/resume).
    pub enabled: bool,
    /// Free-space floor of a filesystem; below it the disk is `low`.
    pub min_free_bytes: u64,
    /// Recovery margin above `min_free_bytes`.
    pub margin_bytes: u64,
    /// Poll interval in seconds.
    pub poll_secs: u64,
    /// Add behaviour below `min_free_bytes`.
    pub on_low: LowAction,
    /// Refuses an add whose payload plus the margin does not fit.
    pub require_space_for_torrent: bool,
    /// Library and staging folders.
    pub zone: Vec<Zone>,
}

impl Default for DiskGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_free_bytes: 0,
            margin_bytes: 0,
            poll_secs: 30,
            on_low: LowAction::Reject,
            require_space_for_torrent: false,
            zone: Vec::new(),
        }
    }
}

impl DiskGuardConfig {
    /// Checks the zones and the threshold values.
    pub fn validate(&self) -> Result<(), String> {
        if self.poll_secs == 0 || self.poll_secs > 3_600 {
            return Err("diskguard.poll_secs must be between 1 and 3600".to_owned());
        }
        crate::diskguard::validate_zones(&self.zone)
    }
}

/// The weekly scheduler grid and its limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SchedulerConfig {
    pub enabled: bool,
    /// IANA timezone name, e.g. `Europe/Budapest`.
    pub timezone: String,
    /// Yellow-slot download limit in bytes per second.
    pub download_rate_limit: u64,
    /// Yellow-slot upload limit in bytes per second.
    pub upload_rate_limit: u64,
    /// Whether red slots pause seeding torrents too.
    pub pause_seeding: bool,
    /// Seven rows of 24 `g`/`y`/`r` characters, Monday first.
    pub week: Vec<String>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timezone: "UTC".to_owned(),
            download_rate_limit: 0,
            upload_rate_limit: 0,
            pause_seeding: true,
            week: Grid::default().render(),
        }
    }
}

impl SchedulerConfig {
    /// Checks the timezone and the grid before the config is applied.
    pub fn validate(&self) -> Result<(), String> {
        if self.timezone.trim().is_empty() {
            return Err("scheduler timezone must not be empty".to_owned());
        }
        if self.timezone.trim().parse::<chrono_tz::Tz>().is_err() {
            return Err(format!(
                "unknown scheduler timezone {:?}",
                self.timezone.trim()
            ));
        }
        Grid::parse(&self.week).map_err(|error| format!("scheduler: {error}"))?;
        Ok(())
    }
}

/// Session-wide engine limits. `0` means unlimited.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Download limit in bytes per second; `0` means unlimited.
    pub download_rate_limit: u64,
    /// Upload limit in bytes per second; `0` means unlimited.
    pub upload_rate_limit: u64,
    /// Maximum number of simultaneously downloading torrents; `0` uses the
    /// engine's 20 000 soak head-room.
    pub active_downloads: u32,
    /// Maximum number of simultaneously seeding torrents; `0` uses the engine's
    /// 20 000 soak head-room.
    pub active_seeds: u32,
    /// Maximum number of active torrents; `0` uses the engine's 20 000 soak
    /// head-room.
    pub active_limit: u32,
    /// Treat newly added torrents as complete without an initial file check.
    pub seed_mode: bool,
    /// Trackers added to torrents whose metainfo carries no announce URL.
    /// Existing lists are never overwritten.
    pub default_trackers: Vec<String>,
    /// Incomplete payloads land here and move to their final folder when the
    /// torrent finishes. Empty keeps the label staging zone (or the global
    /// download directory).
    pub temp_dir: Option<String>,
}

/// One RSS/Atom feed. Items whose link is a `.torrent` are added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RssFeedConfig {
    pub url: String,
    /// Label assigned to the added torrent; empty means none.
    pub label: String,
    /// Save path override; empty means the label staging folder or the
    /// daemon's download directory.
    pub save_path: String,
    /// Regex matched against the item title; empty accepts everything.
    pub filter: String,
    pub enabled: bool,
}

impl Default for RssFeedConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            label: String::new(),
            save_path: String::new(),
            filter: String::new(),
            enabled: true,
        }
    }
}

/// RSS watcher. Off by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RssConfig {
    pub enabled: bool,
    /// Poll interval; the daemon clamps it to at least 60 seconds.
    pub poll_secs: u64,
    pub feed: Vec<RssFeedConfig>,
}

impl RssConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && self.feed.is_empty() {
            return Err("rss.enabled needs at least one [[rss.feed]]".to_owned());
        }
        for feed in &self.feed {
            let url = feed.url.trim();
            if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(format!("rss feed url must be http(s): {url:?}"));
            }
            if !feed.filter.trim().is_empty() {
                regex::Regex::new(feed.filter.trim())
                    .map_err(|error| format!("rss filter for {url}: {error}"))?;
            }
        }
        Ok(())
    }
}

/// Queueing behaviour. The queue order itself lives in the daemon, not in
/// libtorrent (see `persist::load_queue_order`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QueueConfig {
    /// Whether the daemon applies the configured active limits.
    pub enabled: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl FileConfig {
    /// Parses and validates a configuration document.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let file: Self = toml::from_str(text).map_err(ConfigError::Parse)?;
        file.validate()?;
        Ok(file)
    }

    /// Validates the values that have constraints beyond their type.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (name, path) in [
            ("data_dir", self.data_dir.as_deref()),
            ("download_dir", self.download_dir.as_deref()),
            ("token_file", self.token_file.as_deref()),
        ] {
            if let Some(path) = path {
                if path.as_os_str().is_empty() {
                    return Err(ConfigError::invalid(format!("{name} must not be empty")));
                }
            }
        }
        if let Some(bind) = self.bind.as_deref() {
            validate_bind(bind)?;
        }
        self.scheduler.validate().map_err(ConfigError::invalid)?;
        self.diskguard.validate().map_err(ConfigError::invalid)?;
        for (index, hook) in self.hook.iter().enumerate() {
            hook.validate()
                .map_err(|message| ConfigError::invalid(format!("hook[{index}]: {message}")))?;
        }
        let mut seen = BTreeSet::new();
        for (index, watch) in self.autoadd.iter().enumerate() {
            watch
                .validate()
                .map_err(|message| ConfigError::invalid(format!("autoadd[{index}]: {message}")))?;
            if !seen.insert(watch.path.clone()) {
                return Err(ConfigError::invalid(format!(
                    "autoadd[{index}]: duplicate watch path {}",
                    watch.path.display()
                )));
            }
        }
        Ok(())
    }

    /// Resolves the file values with environment overrides applied.
    pub fn resolve(&self, env: &BTreeMap<String, String>) -> Result<ResolvedConfig, ConfigError> {
        let mut locked = BTreeSet::new();

        let data_dir = env_path(env, "FERRENT_DATA_DIR")
            .or_else(|| self.data_dir.clone())
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));
        if env.contains_key("FERRENT_DATA_DIR") {
            locked.insert("data_dir".to_owned());
        }

        let download_dir = env_path(env, "FERRENT_DOWNLOAD_DIR")
            .or_else(|| self.download_dir.clone())
            .unwrap_or_else(|| data_dir.join("downloads"));
        if env.contains_key("FERRENT_DOWNLOAD_DIR") {
            locked.insert("download_dir".to_owned());
        }

        let bind = env
            .get("FERRENT_BIND")
            .cloned()
            .or_else(|| self.bind.clone())
            .unwrap_or_else(|| DEFAULT_BIND.to_owned());
        if env.contains_key("FERRENT_BIND") {
            locked.insert("bind".to_owned());
        }
        validate_bind(&bind)?;

        let token_file = self
            .token_file
            .clone()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TOKEN_FILE));

        let resolved = ResolvedConfig {
            data_dir,
            download_dir,
            bind,
            token_file,
            session: self.session.clone(),
            queue: self.queue.clone(),
            autoadd: self.autoadd.clone(),
            hook: self.hook.clone(),
            scheduler: self.scheduler.clone(),
            notify: self.notify.clone(),
            diskguard: self.diskguard.clone(),
            ratio: self.ratio.clone(),
            network: self.network.clone(),
            metrics: self.metrics.clone(),
            compat: self.compat.clone(),
            rss: self.rss.clone(),
            env_locked: locked,
        };
        resolved.validate()?;
        Ok(resolved)
    }
}

/// A resolved configuration with environment overrides applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub bind: String,
    pub token_file: PathBuf,
    pub session: SessionConfig,
    pub queue: QueueConfig,
    pub autoadd: Vec<WatchDir>,
    pub hook: Vec<HookSpec>,
    pub scheduler: SchedulerConfig,
    pub notify: NotifyConfig,
    pub diskguard: DiskGuardConfig,
    pub ratio: RatioConfig,
    pub network: NetworkConfig,
    pub metrics: MetricsConfig,
    pub compat: CompatConfig,
    pub rss: RssConfig,
    /// Keys whose value came from the environment and cannot be changed by
    /// `SetConfig` until the daemon restarts.
    pub env_locked: BTreeSet<String>,
}

impl ResolvedConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        for (name, path) in [
            ("data_dir", Some(self.data_dir.as_path())),
            ("download_dir", Some(self.download_dir.as_path())),
            ("token_file", Some(self.token_file.as_path())),
        ] {
            if let Some(path) = path {
                if path.as_os_str().is_empty() {
                    return Err(ConfigError::invalid(format!("{name} must not be empty")));
                }
            }
        }
        validate_bind(&self.bind)?;
        if let Some(temp) = &self.session.temp_dir {
            let temp = temp.trim();
            if !temp.is_empty() && !std::path::Path::new(temp).is_absolute() {
                return Err(ConfigError::invalid(format!(
                    "session.temp_dir must be an absolute path: {temp:?}"
                )));
            }
        }
        self.diskguard.validate().map_err(ConfigError::invalid)?;
        self.ratio.validate().map_err(ConfigError::invalid)?;
        self.network.validate().map_err(ConfigError::invalid)?;
        self.metrics.validate().map_err(ConfigError::invalid)?;
        self.compat
            .qbittorrent
            .validate()
            .map_err(ConfigError::invalid)?;
        self.rss.validate().map_err(ConfigError::invalid)?;
        Ok(())
    }
}

fn env_path(env: &BTreeMap<String, String>, key: &str) -> Option<PathBuf> {
    env.get(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn validate_bind(bind: &str) -> Result<(), ConfigError> {
    let bind = bind.trim();
    if bind.is_empty() {
        return Err(ConfigError::invalid("bind must not be empty"));
    }
    let Some((host, port)) = bind.rsplit_once(':') else {
        return Err(ConfigError::invalid(format!(
            "bind {bind:?} must be host:port"
        )));
    };
    if host.trim().is_empty() {
        return Err(ConfigError::invalid(format!(
            "bind {bind:?} must have a host"
        )));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| ConfigError::invalid(format!("bind {bind:?} has an invalid port")))?;
    if port == 0 {
        return Err(ConfigError::invalid(format!(
            "bind {bind:?} must not use port 0"
        )));
    }
    Ok(())
}

/// Reads the API token from a file, trimming the trailing newline.
pub fn read_token_file(path: &Path) -> std::io::Result<String> {
    let token = std::fs::read_to_string(path)?;
    Ok(token.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn legacy_flat_config_parses() {
        let file = FileConfig::parse(
            r#"
data_dir = "/srv/ferrent"
download_dir = "/mnt/downloads"
bind = "0.0.0.0:50051"
token_file = "/etc/ferrent/token"
"#,
        )
        .expect("legacy config must parse");

        assert_eq!(file.data_dir.as_deref(), Some(Path::new("/srv/ferrent")));
        assert_eq!(file.session, SessionConfig::default());
        assert!(file.queue.enabled);
    }

    #[test]
    fn default_trackers_are_parsed() {
        let file = FileConfig::parse(
            r#"
[session]
default_trackers = ["udp://tracker.example:6969/announce", "https://t.example/a"]
"#,
        )
        .expect("default trackers");
        let resolved = file
            .resolve(&std::collections::BTreeMap::new())
            .expect("resolve");
        assert_eq!(
            resolved.session.default_trackers,
            vec![
                "udp://tracker.example:6969/announce".to_owned(),
                "https://t.example/a".to_owned()
            ]
        );
    }

    #[test]
    fn proxy_settings_are_validated() {
        let file = FileConfig::parse(
            r#"
[network]
proxy_type = "socks5"
proxy_host = "127.0.0.1"
proxy_port = 1080
encryption = "forced"
"#,
        )
        .expect("proxy config");
        let resolved = file
            .resolve(&std::collections::BTreeMap::new())
            .expect("resolve");
        assert_eq!(resolved.network.proxy_type, "socks5");
        assert_eq!(resolved.network.proxy_port, 1080);
        assert_eq!(resolved.network.encryption, "forced");

        let error = FileConfig::parse(
            r#"
[network]
proxy_type = "socks5"
"#,
        )
        .expect("parses")
        .resolve(&std::collections::BTreeMap::new())
        .expect_err("a proxy without a host");
        assert!(error.to_string().contains("proxy_host"), "{error}");

        let error = FileConfig::parse(
            r#"
[network]
proxy_type = "socks9"
proxy_host = "h"
proxy_port = 1
"#,
        )
        .expect("parses")
        .resolve(&std::collections::BTreeMap::new())
        .expect_err("unknown proxy type");
        assert!(error.to_string().contains("proxy_type"), "{error}");

        let error = FileConfig::parse("[network]\nencryption = \"maybe\"\n")
            .expect("parses")
            .resolve(&std::collections::BTreeMap::new())
            .expect_err("unknown encryption");
        assert!(error.to_string().contains("encryption"), "{error}");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let error = FileConfig::parse(
            r#"
[session]
download_rate_limit = 1024
nope = 1
"#,
        )
        .expect_err("unknown keys must fail");

        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn invalid_bind_is_rejected() {
        let error = FileConfig::parse("bind = \"localhost\"").expect_err("bind must have a port");
        assert!(error.to_string().contains("host:port"), "{error}");
    }

    #[test]
    fn env_overrides_file_and_is_locked() {
        let file = FileConfig::parse(
            r#"
data_dir = "/from/file"
bind = "127.0.0.1:50051"
"#,
        )
        .expect("config");

        let resolved = file
            .resolve(&env(&[
                ("FERRENT_DATA_DIR", "/from/env"),
                ("FERRENT_BIND", "0.0.0.0:6000"),
            ]))
            .expect("resolve");

        assert_eq!(resolved.data_dir, PathBuf::from("/from/env"));
        assert_eq!(resolved.bind, "0.0.0.0:6000");
        assert_eq!(
            resolved.download_dir,
            PathBuf::from("/from/env").join("downloads")
        );
        assert!(resolved.env_locked.contains("data_dir"));
        assert!(resolved.env_locked.contains("bind"));
        assert!(!resolved.env_locked.contains("download_dir"));
    }

    #[test]
    fn autoadd_tables_parse_and_validate() {
        let file = FileConfig::parse(
            r#"
[[autoadd]]
path = "/data/watch"
label = "movies"
save_path = "/data/media/movies"
on_success = "move"
move_to = "/data/watch/done"

[[autoadd]]
path = "/data/watch2"
add_paused = true
"#,
        )
        .expect("autoadd config");

        assert_eq!(file.autoadd.len(), 2);
        assert_eq!(file.autoadd[0].label.as_deref(), Some("movies"));
        assert_eq!(file.autoadd[0].on_success, crate::autoadd::OnSuccess::Move);
        assert!(file.autoadd[1].add_paused);
        assert_eq!(
            file.autoadd[1].on_success,
            crate::autoadd::OnSuccess::Delete
        );

        let resolved = file.resolve(&env(&[])).expect("resolve");
        assert_eq!(resolved.autoadd.len(), 2);
    }

    #[test]
    fn autoadd_errors_are_reported_with_the_index() {
        let error = FileConfig::parse(
            r#"
[[autoadd]]
path = "/data/watch"
on_success = "move"
"#,
        )
        .expect_err("move without a destination");
        assert!(error.to_string().contains("autoadd[0]"), "{error}");

        let error = FileConfig::parse(
            r#"
[[autoadd]]
path = "/data/watch"

[[autoadd]]
path = "/data/watch"
"#,
        )
        .expect_err("duplicate watch path");
        assert!(
            error.to_string().contains("duplicate watch path"),
            "{error}"
        );
    }

    #[test]
    fn hook_tables_parse_with_the_default_argv() {
        let file = FileConfig::parse(
            r#"
[[hook]]
event = "finished"
command = "/usr/local/bin/notify"

[[hook]]
event = "added"
command = "/bin/echo"
args = ["{id}", "{name}"]
shell = true
timeout_secs = 5
enabled = false
"#,
        )
        .expect("hook config");

        assert_eq!(file.hook.len(), 2);
        assert_eq!(file.hook[0].event, crate::events::EventKind::Finished);
        assert_eq!(file.hook[0].args.len(), 5);
        assert_eq!(file.hook[0].timeout_secs, 60);
        assert!(file.hook[0].enabled);
        assert!(file.hook[1].shell);
        assert!(!file.hook[1].enabled);
        assert_eq!(file.hook[1].timeout_secs, 5);
    }

    #[test]
    fn hook_validation_reports_the_index() {
        let error = FileConfig::parse(
            r#"
[[hook]]
event = "finished"
"#,
        )
        .expect_err("empty command");
        assert!(error.to_string().contains("hook[0]"), "{error}");

        let error = FileConfig::parse(
            r#"
[[hook]]
event = "sometimes"
command = "/bin/true"
"#,
        )
        .expect_err("unknown event");
        assert!(error.to_string().contains("sometimes"), "{error}");
    }

    #[test]
    fn download_dir_defaults_under_data_dir() {
        let resolved = FileConfig::default().resolve(&env(&[])).expect("resolve");
        assert_eq!(resolved.data_dir, PathBuf::from(DEFAULT_DATA_DIR));
        assert_eq!(
            resolved.download_dir,
            PathBuf::from(DEFAULT_DATA_DIR).join("downloads")
        );
        assert_eq!(resolved.bind, DEFAULT_BIND);
        assert_eq!(resolved.token_file, PathBuf::from(DEFAULT_TOKEN_FILE));
    }

    #[test]
    fn diskguard_zones_parse_and_resolve() {
        let file = FileConfig::parse(
            r#"
[diskguard]
min_free_bytes = 1073741824
margin_bytes = 1073741824
poll_secs = 60

[[diskguard.zone]]
path = "/mnt/usbhdd2/Filmek"
label = "Filmek"

[[diskguard.zone]]
path = "/mnt/usbhdd2/_progress"
staging = true
"#,
        )
        .expect("diskguard config");

        assert_eq!(file.diskguard.zone.len(), 2);
        assert_eq!(file.diskguard.zone[0].label.as_deref(), Some("Filmek"));
        assert!(file.diskguard.zone[1].staging);

        let resolved = file.resolve(&env(&[])).expect("resolve");
        assert_eq!(resolved.diskguard.min_free_bytes, 1 << 30);
        assert_eq!(resolved.diskguard.poll_secs, 60);
    }

    #[test]
    fn diskguard_zone_errors_are_reported() {
        let error = FileConfig::parse(
            r#"
[[diskguard.zone]]
path = "/mnt/a/Filmek"
label = "Filmek"

[[diskguard.zone]]
path = "/mnt/b/Filmek"
label = "filmek"
"#,
        )
        .expect_err("duplicate label");
        assert!(error.to_string().contains("filmek"), "{error}");

        let error = FileConfig::parse(
            r#"
[[diskguard.zone]]
path = "/mnt/a/_progress"
staging = true
label = "Filmek"
"#,
        )
        .expect_err("staging with label");
        assert!(error.to_string().contains("staging"), "{error}");

        let error = FileConfig::parse(
            r#"
[[diskguard.zone]]
path = "relative/Filmek"
label = "Filmek"
"#,
        )
        .expect_err("relative path");
        assert!(error.to_string().contains("absolute"), "{error}");
    }
}
