//! Revisioned, atomic configuration snapshots.
//!
//! A `SetConfig` first parses and validates the whole document, then checks the
//! client's revision, and only then swaps the live snapshot. A failed parse or
//! a stale revision leaves the previous snapshot untouched.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;

use super::ConfigError;
use super::schema::{FileConfig, ResolvedConfig};

#[derive(Debug)]
struct Snapshot {
    file: FileConfig,
    revision: u64,
}

/// Holds the current configuration and its revision.
#[derive(Debug)]
pub struct ConfigStore {
    current: ArcSwap<Snapshot>,
    path: PathBuf,
}

impl ConfigStore {
    /// Loads a configuration file, or the defaults when it does not exist.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        let file = if path.is_file() {
            let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
                path: path.clone(),
                source,
            })?;
            FileConfig::parse(&text)?
        } else {
            FileConfig::default()
        };
        Ok(Self {
            current: ArcSwap::from_pointee(Snapshot { file, revision: 0 }),
            path,
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
        let next = FileConfig::parse(text)?;
        let current = self.current.load();
        if current.revision != expected_revision {
            return Err(ConfigError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let revision = current.revision + 1;
        drop(current);
        self.current.store(Arc::new(Snapshot {
            file: next,
            revision,
        }));
        Ok(revision)
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
        self.apply_toml(&text, expected_revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn store() -> ConfigStore {
        ConfigStore::load("/nonexistent/ferrent/config.toml").expect("defaults")
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
