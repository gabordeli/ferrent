//! Typed daemon configuration.
//!
//! [`ConfigStore`] holds a revisioned snapshot; [`ResolvedConfig`] applies the
//! process environment on top of the file. A failed parse or a stale revision
//! never replaces the live snapshot.

mod apply;
pub mod schema;

use std::fmt;
use std::path::PathBuf;

pub use apply::ConfigStore;
pub use schema::{
    DiskGuardConfig, FileConfig, LowAction, NotifyConfig, QueueConfig, ResolvedConfig,
    SessionConfig, read_token_file,
};

/// A configuration read, parse, validation or revision error.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The TOML document is malformed or has an unknown key.
    Parse(toml::de::Error),
    /// A value failed validation.
    Invalid(String),
    /// The client sent a stale revision; the live snapshot is unchanged.
    RevisionConflict { expected: u64, actual: u64 },
}

impl ConfigError {
    /// Builds an [`ConfigError::Invalid`] from a message.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "cannot read config {}: {source}", path.display())
            }
            Self::Parse(error) => write!(f, "invalid config: {error}"),
            Self::Invalid(message) => write!(f, "invalid config: {message}"),
            Self::RevisionConflict { expected, actual } => write!(
                f,
                "config revision {expected} is stale; current revision is {actual}"
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse(error) => Some(error),
            Self::Invalid(_) | Self::RevisionConflict { .. } => None,
        }
    }
}
