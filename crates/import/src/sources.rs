//! Import sources.
//!
//! A client is either read from its state files (Deluge keeps them under
//! `state/`) or queried over its own API (Transmission, uTorrent, ...). The
//! daemon only needs a [`Source`]: what it reads and how is the adapter's
//! business, so a new client is one module and one registry entry here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{Options, Report, default_config_dir, scan};

/// One client the daemon can import from.
pub trait Source: Send {
    /// Stable identifier used by the RPC and the CLI (`deluge`, ...).
    fn id(&self) -> &'static str;
    /// Reads the client's torrents without touching anything.
    fn scan(&self) -> Result<Report, String>;
}

/// Deluge's state directory (offline files, no running daemon needed).
pub struct Deluge {
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
}

impl Source for Deluge {
    fn id(&self) -> &'static str {
        "deluge"
    }

    fn scan(&self) -> Result<Report, String> {
        scan(&Options {
            state_dir: self.state_dir.clone(),
            config_dir: self.config_dir.clone(),
        })
    }
}

/// Builds the adapter named by `id` from request options.
///
/// The common keys are `state_dir` and `config_dir`; a client speaking an API
/// will read its own (`url`, `user`, `password`, `api_key`, ...) from the same
/// map, so the RPC does not grow a field per client.
pub fn build(id: &str, options: &BTreeMap<String, String>) -> Result<Box<dyn Source>, String> {
    let value = |key: &str| {
        options
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    match id.trim() {
        "" | "deluge" => {
            let state_dir = value("state_dir").ok_or("state_dir is required")?;
            let state = Path::new(state_dir);
            let config_dir =
                value("config_dir").map_or_else(|| default_config_dir(state), PathBuf::from);
            Ok(Box::new(Deluge {
                state_dir: state.to_path_buf(),
                config_dir,
            }))
        }
        other => Err(format!("unsupported import source {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(state_dir: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("state_dir".to_owned(), state_dir.to_owned())])
    }

    #[test]
    fn deluge_is_the_default_and_needs_a_state_dir() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/deluge-state");
        let source = build("", &options(&fixture.display().to_string())).expect("source");
        assert_eq!(source.id(), "deluge");
        let report = source.scan().expect("scan");
        assert_eq!(report.torrents.len(), 2);

        assert!(build("deluge", &BTreeMap::new()).is_err());
        assert_eq!(
            build("transmission", &options("/tmp")).err().as_deref(),
            Some("unsupported import source \"transmission\"")
        );
    }

    #[test]
    fn the_config_dir_defaults_from_the_state_dir() {
        let state =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/deluge-state");
        let source = build("deluge", &options(&state.display().to_string())).expect("source");
        // The fixture keeps the plugin configs next to the torrents, which is
        // what `default_config_dir` must pick for the labels to come through.
        let report = source.scan().expect("scan");
        assert!(
            report
                .plugins
                .labels
                .iter()
                .any(|label| label.name == "movies")
        );
    }
}
