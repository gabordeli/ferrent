//! Imports a Deluge state directory: the torrents plus the bundled plugin
//! configs (label, autoadd, execute, scheduler).
//!
//! Deluge stores one `.torrent` per torrent under `state/` and keeps the
//! plugin data as JSON next to it. Nothing here talks to the daemon: [`scan`]
//! builds a report, and the caller decides whether to add the torrents. Both
//! the CLI and the daemon's import RPC use this crate, so the preview and the
//! commit can never disagree.

mod bencode;
mod sources;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

pub use sources::{Deluge, Source, build as build_source};

/// One torrent found in the state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub info_hash: String,
    pub name: String,
    pub total_bytes: u64,
    pub label: Option<String>,
    /// Announce URLs from the metainfo, for the import filters.
    pub trackers: Vec<String>,
    /// Deluge's save path for this torrent, from `torrents.fastresume`.
    /// `None` when the resume state has no entry (a fresh or broken torrent).
    pub save_path: Option<String>,
    /// libtorrent resume data, so an import does not have to recheck the
    /// already-downloaded payload.
    pub resume_data: Option<Vec<u8>>,
}

/// A Deluge AutoAdd watch directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchDir {
    pub path: String,
    pub label: Option<String>,
    pub download_location: Option<String>,
    pub enabled: bool,
}

/// A label declared by the Label plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredLabel {
    pub name: String,
    pub color: String,
}

/// An Execute plugin command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub event: String,
    pub command: String,
}

/// What the plugin configs say, as far as the import is concerned.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plugins {
    /// infohash → label name, from `label.conf`.
    pub assignments: BTreeMap<String, String>,
    /// Declared labels, from `label.conf`.
    pub labels: Vec<DeclaredLabel>,
    pub watch_dirs: Vec<WatchDir>,
    pub commands: Vec<Command>,
    pub scheduler_cells: usize,
    pub scheduler_red: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub torrents: Vec<Found>,
    pub plugins: Plugins,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub state_dir: PathBuf,
    /// Where `label.conf` and friends live; Deluge keeps them next to `state/`.
    pub config_dir: PathBuf,
}

/// Where the plugin configs live when the caller did not say.
///
/// Deluge keeps `state/` inside its config directory, so the parent is the
/// usual place; an older layout (or a hand-made fixture) has them next to the
/// torrents, which wins when any of the four files is there.
#[must_use]
pub fn default_config_dir(state_dir: &Path) -> PathBuf {
    const FILES: [&str; 4] = [
        "label.conf",
        "autoadd.conf",
        "execute.conf",
        "scheduler.conf",
    ];
    if FILES.iter().any(|name| state_dir.join(name).is_file()) {
        return state_dir.to_path_buf();
    }
    state_dir
        .parent()
        .map_or_else(|| state_dir.to_path_buf(), Path::to_path_buf)
}

/// Reads the state directory and the plugin configs without touching anything.
pub fn scan(options: &Options) -> Result<Report, String> {
    let mut warnings = Vec::new();
    let plugins = read_plugins(&options.config_dir, &mut warnings);

    let mut torrents = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in read_dir(&options.state_dir, &mut warnings) {
        if entry.extension().and_then(|name| name.to_str()) != Some("torrent") {
            continue;
        }
        match std::fs::read(&entry) {
            Ok(data) => match bencode::parse(&data) {
                Ok(metainfo) => {
                    if !seen.insert(metainfo.info_hash.clone()) {
                        warnings.push(format!(
                            "{}: duplicate infohash {}",
                            entry.display(),
                            metainfo.info_hash
                        ));
                        continue;
                    }
                    torrents.push(Found {
                        label: plugins.assignments.get(&metainfo.info_hash).cloned(),
                        path: entry,
                        info_hash: metainfo.info_hash,
                        name: metainfo.name,
                        total_bytes: metainfo.total_bytes,
                        trackers: metainfo.trackers,
                        save_path: None,
                        resume_data: None,
                    });
                }
                Err(error) => warnings.push(format!("{}: {error}", entry.display())),
            },
            Err(error) => warnings.push(format!("{}: {error}", entry.display())),
        }
    }
    torrents.sort_by(|left, right| left.name.cmp(&right.name));

    let mut resumes = read_fastresume(&options.state_dir, &mut warnings);
    for torrent in &mut torrents {
        if let Some(resume) = resumes.remove(&torrent.info_hash) {
            torrent.save_path = resume.save_path;
            torrent.resume_data = Some(resume.data);
        }
    }
    if !resumes.is_empty() {
        warnings.push(format!(
            "{} fastresume entries match no torrent in the state directory",
            resumes.len()
        ));
    }

    let known: BTreeSet<&String> = plugins.labels.iter().map(|label| &label.name).collect();
    for (hash, label) in &plugins.assignments {
        if !known.contains(label) {
            warnings.push(format!(
                "{hash}: label {label:?} is not declared in label.conf"
            ));
        }
    }
    for dir in &plugins.watch_dirs {
        if !Path::new(&dir.path).is_dir() {
            warnings.push(format!(
                "autoadd watch dir is not on this host: {}",
                dir.path
            ));
        }
        if let Some(location) = &dir.download_location {
            if !Path::new(location).is_dir() {
                warnings.push(format!(
                    "autoadd download location is not on this host: {location}"
                ));
            }
        }
    }

    Ok(Report {
        torrents,
        plugins,
        warnings,
    })
}

fn read_dir(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect(),
        Err(error) => {
            warnings.push(format!("{}: {error}", dir.display()));
            Vec::new()
        }
    }
}

/// One torrent's resume state as far as the import needs it.
struct Resume {
    save_path: Option<String>,
    data: Vec<u8>,
}

/// Reads `state/torrents.fastresume`: info-hash → libtorrent resume blob.
///
/// The `.torrent` files under `state/` carry no save path; the resume blob
/// does, and it is also what lets libtorrent pick up the finished payload
/// without rechecking it. Without this every imported torrent lands in the
/// session download directory and its data looks missing.
fn read_fastresume(state_dir: &Path, warnings: &mut Vec<String>) -> BTreeMap<String, Resume> {
    let path = state_dir.join("torrents.fastresume");
    let Ok(data) = std::fs::read(&path) else {
        return BTreeMap::new();
    };
    let value = match bencode::parse_value(&data) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("{}: {error}", path.display()));
            return BTreeMap::new();
        }
    };
    let bencode::Value::Dict(entries) = value else {
        warnings.push(format!("{}: not a dictionary", path.display()));
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (hash, value) in entries {
        let bencode::Value::Bytes(blob) = value else {
            continue;
        };
        let save_path = match bencode::parse_value(&blob) {
            Ok(resume) => resume
                .get(b"save_path")
                .and_then(bencode::Value::as_bytes)
                .map(|path| String::from_utf8_lossy(path).into_owned())
                .filter(|path| !path.trim().is_empty()),
            Err(error) => {
                warnings.push(format!(
                    "{}: {}: {error}",
                    path.display(),
                    String::from_utf8_lossy(&hash)
                ));
                None
            }
        };
        out.insert(
            String::from_utf8_lossy(&hash).to_ascii_lowercase(),
            Resume {
                save_path,
                data: blob,
            },
        );
    }
    out
}

fn read_plugins(config_dir: &Path, warnings: &mut Vec<String>) -> Plugins {
    let mut plugins = Plugins::default();
    if let Some(root) = read_json(&config_dir.join("label.conf"), warnings) {
        if let Some(labels) = root.get("labels").and_then(Json::as_object) {
            for label in labels.values() {
                let name = label
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if name.is_empty() {
                    continue;
                }
                plugins.labels.push(DeclaredLabel {
                    color: label
                        .get("color")
                        .and_then(Json::as_str)
                        .unwrap_or("#2f81f7")
                        .to_owned(),
                    name,
                });
            }
        }
        if let Some(map) = root.get("torrent_labels").and_then(Json::as_object) {
            for (hash, label) in map {
                if let Some(label) = label.as_str() {
                    plugins
                        .assignments
                        .insert(hash.to_ascii_lowercase(), label.to_owned());
                }
            }
        }
    }
    if let Some(root) = read_json(&config_dir.join("autoadd.conf"), warnings) {
        if let Some(dirs) = root.get("watchdirs").and_then(Json::as_object) {
            for dir in dirs.values() {
                plugins.watch_dirs.push(WatchDir {
                    path: dir
                        .get("path")
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    label: dir.get("label").and_then(Json::as_str).map(str::to_owned),
                    download_location: dir
                        .get("download_location")
                        .and_then(Json::as_str)
                        .map(str::to_owned),
                    enabled: dir.get("enabled").and_then(Json::as_bool).unwrap_or(true),
                });
            }
        }
    }
    if let Some(root) = read_json(&config_dir.join("execute.conf"), warnings) {
        if let Some(commands) = root.get("commands").and_then(Json::as_array) {
            for command in commands {
                let parts: Vec<&Json> = command
                    .as_array()
                    .map(|list| list.iter().collect())
                    .unwrap_or_default();
                let event = parts.get(1).and_then(|value| value.as_str());
                let text = parts.get(2).and_then(|value| value.as_str());
                if let (Some(event), Some(text)) = (event, text) {
                    plugins.commands.push(Command {
                        event: event.to_owned(),
                        command: text.to_owned(),
                    });
                }
            }
        }
    }
    if let Some(root) = read_json(&config_dir.join("scheduler.conf"), warnings) {
        if let Some(grid) = root.get("button_state").and_then(Json::as_array) {
            for row in grid {
                if let Some(row) = row.as_array() {
                    plugins.scheduler_cells += row.len();
                    plugins.scheduler_red +=
                        row.iter().filter(|cell| cell.as_str() == Some("r")).count();
                }
            }
        }
    }
    plugins
}

fn read_json(path: &Path, warnings: &mut Vec<String>) -> Option<Json> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(format!("{}: {error}", path.display()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Options {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/deluge-state");
        Options {
            state_dir: dir.clone(),
            config_dir: dir,
        }
    }

    #[test]
    fn scans_torrents_and_plugin_configs() {
        let report = scan(&fixture()).expect("scan");
        assert_eq!(report.torrents.len(), 2);
        let alpha = &report.torrents[0];
        assert_eq!(alpha.name, "alpha.bin");
        assert_eq!(alpha.total_bytes, 1000);
        assert_eq!(alpha.label.as_deref(), Some("movies"));
        let names: Vec<&str> = report
            .plugins
            .labels
            .iter()
            .map(|label| label.name.as_str())
            .collect();
        assert_eq!(names, ["movies", "music"]);
        assert_eq!(report.plugins.watch_dirs.len(), 1);
        assert_eq!(
            report.plugins.watch_dirs[0].label.as_deref(),
            Some("movies")
        );
        assert_eq!(report.plugins.commands.len(), 2);
        assert_eq!(report.plugins.scheduler_cells, 24 * 7);
        assert_eq!(report.plugins.scheduler_red, 6 * 7);
    }

    #[test]
    fn reports_host_dependent_paths() {
        let report = scan(&fixture()).expect("scan");
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("/srv/watch/incoming"))
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("/srv/downloads/incoming"))
        );
    }

    #[test]
    fn the_fastresume_carries_the_save_path_and_the_resume_blob() {
        let report = scan(&fixture()).expect("scan");
        let alpha = report
            .torrents
            .iter()
            .find(|torrent| torrent.name == "alpha.bin")
            .expect("alpha");
        assert_eq!(alpha.save_path.as_deref(), Some("/srv/movies"));
        assert!(
            alpha
                .resume_data
                .as_ref()
                .is_some_and(|data| !data.is_empty()),
            "the resume blob is carried so an import skips the recheck"
        );

        let beta = report
            .torrents
            .iter()
            .find(|torrent| torrent.name == "beta.bin")
            .expect("beta");
        assert_eq!(beta.save_path, None, "no resume entry means no path");
        assert_eq!(beta.resume_data, None);

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("fastresume entries match no torrent")),
            "an orphaned resume entry is reported: {:?}",
            report.warnings
        );
    }

    #[test]
    fn missing_state_dir_is_a_warning_not_a_failure() {
        let options = Options {
            state_dir: PathBuf::from("/nonexistent/deluge/state"),
            config_dir: PathBuf::from("/nonexistent/deluge"),
        };
        let report = scan(&options).expect("scan");
        assert!(report.torrents.is_empty());
        assert_eq!(report.warnings.len(), 1);
    }
}
