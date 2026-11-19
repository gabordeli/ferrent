//! Revisioned, atomic configuration snapshots.
//!
//! A `SetConfig` first parses and validates the whole document, then checks the
//! client's revision, and only then swaps the live snapshot. A failed parse or
//! a stale revision leaves the previous snapshot untouched.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;

use super::ConfigError;
use super::schema::{FileConfig, ResolvedConfig};

#[derive(Debug)]
struct Snapshot {
    file: FileConfig,
    raw: String,
    revision: u64,
}

/// Holds the current configuration and its revision.
#[derive(Debug)]
pub struct ConfigStore {
    current: ArcSwap<Snapshot>,
    path: PathBuf,
    apply_lock: Mutex<()>,
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self {
            current: ArcSwap::from_pointee(Snapshot {
                file: FileConfig::default(),
                raw: String::new(),
                revision: 0,
            }),
            path: PathBuf::new(),
            apply_lock: Mutex::new(()),
        }
    }
}

impl ConfigStore {
    /// Loads a configuration file, or the defaults when it does not exist.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let raw = if path.is_file() {
            std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
                path: path.clone(),
                source,
            })?
        } else {
            String::new()
        };
        let file = FileConfig::parse(&raw)?;
        Ok(Self {
            current: ArcSwap::from_pointee(Snapshot {
                file,
                raw,
                revision: 0,
            }),
            path,
            apply_lock: Mutex::new(()),
        })
    }

    /// The configuration file path.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The current file configuration.
    #[must_use]
    pub fn file(&self) -> FileConfig {
        self.current.load().file.clone()
    }

    /// The raw TOML document, preserving comments and formatting.
    #[must_use]
    pub fn toml(&self) -> String {
        self.current.load().raw.clone()
    }

    /// The current revision, echoed by `GetConfig` and required by `SetConfig`.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.current.load().revision
    }

    /// Resolves the current snapshot with the process environment.
    pub fn resolved(&self) -> Result<ResolvedConfig, ConfigError> {
        let env = std::env::vars().collect();
        self.current.load().file.resolve(&env)
    }

    /// Applies a complete document if the client's revision is current.
    ///
    /// Returns the new revision. The live snapshot is replaced only after the
    /// document parsed and validated.
    pub fn apply_toml(&self, text: &str, expected_revision: u64) -> Result<u64, ConfigError> {
        self.apply(text, expected_revision, true)
    }

    fn apply(&self, text: &str, expected_revision: u64, persist: bool) -> Result<u64, ConfigError> {
        let _guard = self
            .apply_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let next = FileConfig::parse(text)?;
        let current = self.current.load();
        if current.revision != expected_revision {
            return Err(ConfigError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        if persist {
            self.persist(text)?;
        }
        let revision = current.revision + 1;
        drop(current);
        self.current.store(Arc::new(Snapshot {
            file: next,
            raw: text.to_owned(),
            revision,
        }));
        Ok(revision)
    }

    /// Writes the document atomically next to the configured path.
    ///
    /// The write happens before the in-memory swap, so a read-only config file
    /// or a full disk leaves the running snapshot unchanged.
    fn persist(&self, text: &str) -> Result<(), ConfigError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: self.path.clone(),
            source,
        })?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.toml");
        let temporary = parent.join(format!(".{name}.tmp.{}", std::process::id()));
        std::fs::write(&temporary, text).map_err(|source| ConfigError::Io {
            path: self.path.clone(),
            source,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&self.path)
                .map(|metadata| metadata.permissions().mode())
                .unwrap_or(0o640);
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)).map_err(
                |source| ConfigError::Io {
                    path: self.path.clone(),
                    source,
                },
            )?;
        }
        std::fs::rename(&temporary, &self.path).map_err(|source| ConfigError::Io {
            path: self.path.clone(),
            source,
        })
    }

    /// Re-reads the file and applies it as a new revision.
    pub fn reload(&self, expected_revision: u64) -> Result<u64, ConfigError> {
        let text = if self.path.is_file() {
            std::fs::read_to_string(&self.path).map_err(|source| ConfigError::Io {
                path: self.path.clone(),
                source,
            })?
        } else {
            String::new()
        };
        self.apply(&text, expected_revision, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn store() -> ConfigStore {
        ConfigStore::default()
    }

    #[test]
    fn apply_bumps_revision_and_changes_values() {
        let store = store();
        assert_eq!(store.revision(), 0);

        let revision = store
            .apply_toml("bind = \"0.0.0.0:6000\"\n", 0)
            .expect("apply");

        assert_eq!(revision, 1);
        assert_eq!(store.revision(), 1);
        assert_eq!(store.file().bind.as_deref(), Some("0.0.0.0:6000"));
    }

    #[test]
    fn raw_toml_is_preserved() {
        let store = store();
        let text = "# keep this comment\nbind = \"0.0.0.0:6000\"\n";
        store.apply_toml(text, 0).expect("apply");
        assert_eq!(store.toml(), text);
    }

    #[test]
    fn stale_revision_is_rejected_without_a_swap() {
        let store = store();
        store
            .apply_toml("bind = \"0.0.0.0:6000\"\n", 0)
            .expect("apply");

        let error = store
            .apply_toml("bind = \"0.0.0.0:7000\"\n", 0)
            .expect_err("stale revision");

        assert!(matches!(
            error,
            ConfigError::RevisionConflict {
                expected: 0,
                actual: 1
            }
        ));
        assert_eq!(store.file().bind.as_deref(), Some("0.0.0.0:6000"));
        assert_eq!(store.revision(), 1);
    }

    #[test]
    fn failed_apply_keeps_the_previous_snapshot() {
        let store = store();
        store
            .apply_toml("bind = \"0.0.0.0:6000\"\n", 0)
            .expect("apply");

        store
            .apply_toml("[session]\nnope = 1\n", 1)
            .expect_err("invalid config");

        assert_eq!(store.revision(), 1);
        assert_eq!(store.file().bind.as_deref(), Some("0.0.0.0:6000"));
    }

    #[test]
    fn apply_persists_and_survives_reload() {
        let path = std::env::temp_dir().join(format!(
            "ferrent-config-persist-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, "bind = \"0.0.0.0:6100\"\n").expect("write");

        let store = ConfigStore::load(&path).expect("load");
        let text = "# persisted\nbind = \"0.0.0.0:6300\"\n";
        store.apply_toml(text, 0).expect("apply");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), text);

        let reloaded = ConfigStore::load(&path).expect("reload");
        assert_eq!(reloaded.file().bind.as_deref(), Some("0.0.0.0:6300"));
        assert_eq!(reloaded.revision(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failed_apply_does_not_touch_the_file() {
        let path = std::env::temp_dir().join(format!(
            "ferrent-config-keep-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let text = "bind = \"0.0.0.0:6100\"\n";
        std::fs::write(&path, text).expect("write");
        let store = ConfigStore::load(&path).expect("load");

        store
            .apply_toml("[session]\nnope = 1\n", 0)
            .expect_err("invalid config");

        assert_eq!(std::fs::read_to_string(&path).expect("read"), text);
        assert_eq!(store.revision(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn concurrent_applies_serialize_on_revision() {
        let store = Arc::new(ConfigStore::default());
        let first = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.apply_toml("bind = \"0.0.0.0:6000\"\n", 0))
        };
        let second = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.apply_toml("bind = \"0.0.0.0:7000\"\n", 0))
        };

        let results = [first.join().expect("first"), second.join().expect("second")];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .any(|result| matches!(result, Err(ConfigError::RevisionConflict { .. })))
        );
        assert_eq!(store.revision(), 1);
    }

    #[test]
    fn reload_reads_the_file() {
        let path = std::env::temp_dir().join(format!(
            "ferrent-config-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, "bind = \"0.0.0.0:6100\"\n").expect("write");

        let store = ConfigStore::load(&path).expect("load");
        std::fs::write(&path, "bind = \"0.0.0.0:6200\"\n").expect("rewrite");
        store.reload(0).expect("reload");

        assert_eq!(store.file().bind.as_deref(), Some("0.0.0.0:6200"));
        assert_eq!(store.revision(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
