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
use crate::hooks::HookSpec;
use crate::scheduler::Grid;

const DEFAULT_DATA_DIR: &str = "./ferrent-data";
const DEFAULT_BIND: &str = "127.0.0.1:50051";
const DEFAULT_TOKEN_FILE: &str = "/etc/ferrent/token";

/// The on-disk configuration file.
///
/// Every field has a default so a missing file (or a partial file) is valid.
/// `deny_unknown_fields` makes a typo a startup error rather than a silently
/// ignored setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
            env_locked: locked,
        };
        resolved.validate()?;
        Ok(resolved)
    }
}

/// A resolved configuration with environment overrides applied.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}
