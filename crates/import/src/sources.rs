//! Import sources.
//!
//! A client is either read from its state files (Deluge keeps them under
//! `state/`) or queried over its own API (Transmission, uTorrent, ...). The
//! daemon only needs a [`Source`]: what it reads and how is the adapter's
//! business, so a new client is one module and one registry entry here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::bencode::Value;
use crate::{DeclaredLabel, Found, Options, Plugins, Report, bencode, default_config_dir, scan};

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

/// uTorrent's app-data directory: `resume.dat` plus the `.torrent` files it
/// keeps next to it. Unlike Deluge there is no libtorrent resume blob, so an
/// imported torrent rechecks its payload once.
pub struct UTorrent {
    pub app_dir: PathBuf,
    /// Replaces every save path; for a daemon that sees another filesystem.
    pub save_root: Option<String>,
    /// `(from, to)` prefixes for client paths the daemon can reach under a
    /// different root, e.g. `C:\downloads=/srv/torrents`.
    pub path_map: Vec<(String, String)>,
}

impl Source for UTorrent {
    fn id(&self) -> &'static str {
        "utorrent"
    }

    fn scan(&self) -> Result<Report, String> {
        scan_utorrent(self)
    }
}

fn scan_utorrent(source: &UTorrent) -> Result<Report, String> {
    let path = source.app_dir.join("resume.dat");
    let data = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let value =
        bencode::parse_value(&data).map_err(|error| format!("{}: {error}", path.display()))?;
    let Value::Dict(entries) = value else {
        return Err(format!("{}: not a dictionary", path.display()));
    };

    let mut torrents = Vec::new();
    let mut warnings = Vec::new();
    let mut labels = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for (key, entry) in entries {
        if key == b".fileguard" || key == b"rec" {
            continue;
        }
        let Value::Dict(_) = entry else {
            continue;
        };
        let Some(info) = entry.get(b"info").and_then(Value::as_bytes) else {
            continue;
        };
        if info.len() != 20 {
            warnings.push(format!(
                "{}: resume entry has {} info bytes, not 20",
                String::from_utf8_lossy(&key),
                info.len()
            ));
            continue;
        }
        let name = String::from_utf8_lossy(&key).into_owned();
        let torrent_path = source.app_dir.join(name.replace('\\', "/"));
        let metainfo = match std::fs::read(&torrent_path) {
            Ok(bytes) => match bencode::parse(&bytes) {
                Ok(metainfo) => metainfo,
                Err(error) => {
                    warnings.push(format!("{}: {error}", torrent_path.display()));
                    continue;
                }
            },
            Err(error) => {
                warnings.push(format!("{}: {error}", torrent_path.display()));
                continue;
            }
        };
        if !seen.insert(metainfo.info_hash.clone()) {
            warnings.push(format!("{name}: duplicate infohash {}", metainfo.info_hash));
            continue;
        }
        let resume_hash = hex(info);
        if resume_hash != metainfo.info_hash {
            warnings.push(format!(
                "{name}: resume infohash {resume_hash} does not match the torrent ({})",
                metainfo.info_hash
            ));
        }
        let label = first_label(&entry);
        if let Some(label) = &label {
            labels.insert(label.clone());
        }
        let save_path = entry
            .get(b"path")
            .and_then(Value::as_bytes)
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .filter(|path| !path.trim().is_empty())
            .map(|path| source.map_save_path(&path));

        torrents.push(Found {
            path: torrent_path,
            info_hash: metainfo.info_hash,
            name: metainfo.name,
            total_bytes: metainfo.total_bytes,
            label,
            trackers: metainfo.trackers,
            save_path,
            resume_data: None,
        });
    }
    torrents.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(Report {
        torrents,
        plugins: Plugins {
            labels: labels
                .into_iter()
                .map(|name| DeclaredLabel {
                    name,
                    color: "#2f81f7".to_owned(),
                })
                .collect(),
            ..Plugins::default()
        },
        warnings,
    })
}

impl UTorrent {
    fn map_save_path(&self, raw: &str) -> String {
        if let Some(root) = &self.save_root {
            return root.clone();
        }
        let mut mapped = raw.to_owned();
        for (from, to) in &self.path_map {
            if mapped.len() >= from.len() && mapped[..from.len()].eq_ignore_ascii_case(from) {
                mapped = format!("{to}{}", &mapped[from.len()..]);
                break;
            }
        }
        parent_of(&mapped)
    }
}

/// uTorrent stores the full path of the torrent's root (the file for a
/// single-file torrent, the folder for a multi-file one); libtorrent wants
/// the directory above it.
fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(0) => trimmed[..1].to_owned(),
        Some(index) => trimmed[..index].to_owned(),
        None => trimmed.to_owned(),
    }
}

fn first_label(entry: &Value) -> Option<String> {
    let Value::List(items) = entry.get(b"labels")? else {
        return None;
    };
    items
        .iter()
        .filter_map(Value::as_bytes)
        .map(|label| String::from_utf8_lossy(label).into_owned())
        .find(|label| !label.trim().is_empty())
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
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
        "utorrent" => {
            let app_dir = value("app_dir")
                .or_else(|| value("state_dir"))
                .ok_or("app_dir is required")?;
            let path_map = value("path_map").map_or_else(|| Ok(Vec::new()), parse_path_map)?;
            Ok(Box::new(UTorrent {
                app_dir: PathBuf::from(app_dir),
                save_root: value("save_root").map(str::to_owned),
                path_map,
            }))
        }
        other => Err(format!("unsupported import source {other:?}")),
    }
}

/// Parses `FROM=TO` pairs separated by `;`.
fn parse_path_map(raw: &str) -> Result<Vec<(String, String)>, String> {
    raw.split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (from, to) = pair
                .split_once('=')
                .ok_or_else(|| format!("path_map entry {pair:?} must be FROM=TO"))?;
            let (from, to) = (from.trim(), to.trim());
            if from.is_empty() || to.is_empty() {
                return Err(format!("path_map entry {pair:?} must be FROM=TO"));
            }
            Ok((from.to_owned(), to.to_owned()))
        })
        .collect()
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

    fn utorrent_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/utorrent-state")
    }

    #[test]
    fn utorrent_reads_resume_and_metainfo() {
        let fixture = utorrent_fixture();
        let source = build("utorrent", &options(&fixture.display().to_string())).expect("source");
        assert_eq!(source.id(), "utorrent");
        let report = source.scan().expect("scan");
        assert_eq!(report.torrents.len(), 1);
        let torrent = &report.torrents[0];
        assert_eq!(torrent.name, "sample.bin");
        assert_eq!(
            torrent.info_hash,
            "b954f826ef8d3a4991938dd3f6f8273e2a5c8149"
        );
        assert_eq!(torrent.total_bytes, 1234);
        assert_eq!(torrent.save_path.as_deref(), Some(r"C:\downloads"));
        assert_eq!(torrent.label.as_deref(), Some("test"));
        assert_eq!(torrent.trackers, ["http://tracker.local/announce"]);
        assert!(
            report
                .plugins
                .labels
                .iter()
                .any(|label| label.name == "test"),
            "the resume labels become declared labels"
        );
    }

    #[test]
    fn utorrent_maps_client_paths_for_a_linux_daemon() {
        let fixture = utorrent_fixture();
        let mut options = options(&fixture.display().to_string());
        options.insert(
            "path_map".to_owned(),
            r"C:\downloads=/srv/downloads".to_owned(),
        );
        let report = build("utorrent", &options)
            .expect("source")
            .scan()
            .expect("scan");
        assert_eq!(
            report.torrents[0].save_path.as_deref(),
            Some("/srv/downloads")
        );
    }

    #[test]
    fn utorrent_save_root_overrides_the_resume_path() {
        let fixture = utorrent_fixture();
        let mut options = options(&fixture.display().to_string());
        options.insert("save_root".to_owned(), "/data/torrents".to_owned());
        let report = build("utorrent", &options)
            .expect("source")
            .scan()
            .expect("scan");
        assert_eq!(
            report.torrents[0].save_path.as_deref(),
            Some("/data/torrents")
        );
    }

    #[test]
    fn utorrent_needs_an_app_dir() {
        assert_eq!(
            build("utorrent", &BTreeMap::new()).err().as_deref(),
            Some("app_dir is required")
        );
        assert!(
            build(
                "utorrent",
                &BTreeMap::from([
                    ("app_dir".to_owned(), "/tmp".to_owned()),
                    ("path_map".to_owned(), "broken".to_owned()),
                ])
            )
            .is_err()
        );
    }
}
