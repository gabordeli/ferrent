//! Import page state: what the source returned, the filters over it, and the
//! selection the user is building.
//!
//! The filtering lives here rather than in the daemon so a keystroke does not
//! need a round trip; the candidate list is one `ImportPreview` payload and
//! the page shows a window of it.

use std::collections::BTreeSet;

use ferrent_protocol::v1::ImportTorrentEntry;
use regex::Regex;

/// Rows shown at once. Selecting everything is separate from this window.
pub const PAGE: usize = 100;

/// The text filters over the candidate list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// Regex on the torrent name (empty matches everything).
    pub name: String,
    /// Regex on any announce URL (empty matches everything).
    pub tracker: String,
    /// Exact save path, or `None` for every path.
    pub save_path: Option<String>,
    /// Exact label, or `None` for every label.
    pub label: Option<String>,
    /// Hide the torrents the daemon already has.
    pub only_missing: bool,
}

/// A compiled filter set; an invalid pattern is reported instead of applied.
#[derive(Debug, Clone, Default)]
pub struct Compiled {
    name: Option<Regex>,
    tracker: Option<Regex>,
    pub save_path: Option<String>,
    pub label: Option<String>,
    pub only_missing: bool,
}

impl Filters {
    /// Compiles the patterns. An empty pattern is no filter; an invalid one
    /// is an error the page shows under the input.
    pub fn compile(&self) -> Result<Compiled, String> {
        Ok(Compiled {
            name: optional(&self.name, "name")?,
            tracker: optional(&self.tracker, "tracker")?,
            save_path: self.save_path.clone(),
            label: self.label.clone(),
            only_missing: self.only_missing,
        })
    }
}

fn optional(pattern: &str, what: &str) -> Result<Option<Regex>, String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Ok(None);
    }
    Regex::new(pattern)
        .map(Some)
        .map_err(|error| format!("{what}: {error}"))
}

impl Compiled {
    /// Whether one candidate survives the filters.
    #[must_use]
    pub fn matches(&self, entry: &ImportTorrentEntry) -> bool {
        if self.only_missing && entry.already_present {
            return false;
        }
        if let Some(path) = &self.save_path
            && entry.save_path != *path
        {
            return false;
        }
        if let Some(label) = &self.label
            && entry.label != *label
        {
            return false;
        }
        if let Some(pattern) = &self.name
            && !pattern.is_match(&entry.name)
        {
            return false;
        }
        if let Some(pattern) = &self.tracker
            && !entry.trackers.iter().any(|url| pattern.is_match(url))
        {
            return false;
        }
        true
    }
}

/// One import run's outcome, kept for the footer.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub added: u32,
    pub skipped: u32,
    pub failed: u32,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Everything the import page shows and remembers.
#[derive(Debug, Clone, Default)]
pub struct ImportState {
    pub source: String,
    pub state_dir: String,
    pub config_dir: String,
    pub loading: bool,
    pub running: bool,
    pub error: Option<String>,
    pub entries: Vec<ImportTorrentEntry>,
    pub labels: Vec<String>,
    pub warnings: Vec<String>,
    pub present: u32,
    pub filters: Filters,
    pub filter_error: Option<String>,
    pub selected: BTreeSet<String>,
    /// Index of the first visible row.
    pub offset: usize,
    pub outcome: Option<Outcome>,
}

/// Where Deluge keeps its state on this platform.
///
/// The Debian package's `deluged` writes `/var/lib/deluged/config/state`; the
/// Windows build keeps a roaming profile under `%APPDATA%\deluge\state`. A
/// per-user Linux install edits the field to `~/.config/deluge/state`.
#[cfg(windows)]
fn default_deluge_state_dir() -> String {
    std::env::var("APPDATA").map_or_else(
        |_| r"C:\ProgramData\deluge\state".to_owned(),
        |base| format!("{base}\\deluge\\state"),
    )
}

#[cfg(not(windows))]
fn default_deluge_state_dir() -> String {
    "/var/lib/deluged/config/state".to_owned()
}

impl ImportState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: "deluge".to_owned(),
            state_dir: default_deluge_state_dir(),
            ..Self::default()
        }
    }

    /// Replaces the candidate list; the selection is kept only for hashes
    /// that are still there, so a refresh cannot import a vanished torrent.
    pub fn set_preview(
        &mut self,
        entries: Vec<ImportTorrentEntry>,
        labels: Vec<String>,
        warnings: Vec<String>,
        present: u32,
    ) {
        let kept: BTreeSet<String> = self
            .selected
            .iter()
            .filter(|hash| entries.iter().any(|entry| &entry.info_hash == *hash))
            .cloned()
            .collect();
        self.selected = kept;
        self.entries = entries;
        self.labels = labels;
        self.warnings = warnings;
        self.present = present;
        let filtered = self.filtered().len();
        self.offset = self.offset.min(filtered.saturating_sub(1));
        self.offset = (self.offset / PAGE) * PAGE;
    }

    /// The compiled filters, or `None` while a pattern is invalid.
    #[must_use]
    pub fn compiled(&self) -> Option<Compiled> {
        self.filters.compile().ok()
    }

    /// The candidates that survive the filters, in source order.
    #[must_use]
    pub fn filtered(&self) -> Vec<&ImportTorrentEntry> {
        match self.compiled() {
            Some(compiled) => self
                .entries
                .iter()
                .filter(|entry| compiled.matches(entry))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The window the page draws.
    #[must_use]
    pub fn window(&self) -> (usize, Vec<&ImportTorrentEntry>) {
        let filtered = self.filtered();
        let start = self.offset.min(filtered.len().saturating_sub(1));
        let start = (start / PAGE) * PAGE;
        let end = (start + PAGE).min(filtered.len());
        (start, filtered[start..end].to_vec())
    }

    /// Distinct save paths, for the filter picker.
    #[must_use]
    pub fn save_paths(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for entry in &self.entries {
            if !entry.save_path.trim().is_empty() {
                *counts.entry(entry.save_path.as_str()).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .map(|(path, count)| (path.to_owned(), count))
            .collect()
    }

    /// Distinct labels, for the filter picker.
    #[must_use]
    pub fn used_labels(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for entry in &self.entries {
            if !entry.label.trim().is_empty() {
                *counts.entry(entry.label.as_str()).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .map(|(label, count)| (label.to_owned(), count))
            .collect()
    }

    /// Ticks or unticks one candidate.
    pub fn toggle(&mut self, hash: &str) {
        if !self.selected.remove(hash) {
            self.selected.insert(hash.to_owned());
        }
    }

    /// Ticks every candidate that survives the current filters.
    pub fn select_filtered(&mut self) {
        let hashes: Vec<String> = self
            .filtered()
            .into_iter()
            .map(|entry| entry.info_hash.clone())
            .collect();
        self.selected.extend(hashes);
    }

    /// The hashes to import, in candidate order.
    #[must_use]
    pub fn selection(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| self.selected.contains(&entry.info_hash))
            .map(|entry| entry.info_hash.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, hash: &str, path: &str, tracker: &str) -> ImportTorrentEntry {
        ImportTorrentEntry {
            name: name.to_owned(),
            info_hash: hash.to_owned(),
            total_bytes: 1,
            label: String::new(),
            already_present: false,
            save_path: path.to_owned(),
            trackers: vec![tracker.to_owned()],
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn the_default_state_dir_is_the_debian_one() {
        assert_eq!(
            ImportState::new().state_dir,
            "/var/lib/deluged/config/state"
        );
    }

    fn state() -> ImportState {
        let mut state = ImportState::new();
        state.set_preview(
            vec![
                entry(
                    "Film.2024.mkv",
                    "a",
                    "/mnt/filmek",
                    "http://tracker-a/announce",
                ),
                entry(
                    "Sorozat.S01.mkv",
                    "b",
                    "/mnt/sorozatok",
                    "http://tracker-b/announce",
                ),
                entry(
                    "Film.2025.mkv",
                    "c",
                    "/mnt/filmek",
                    "http://tracker-b/announce",
                ),
            ],
            Vec::new(),
            Vec::new(),
            0,
        );
        state
    }

    #[test]
    fn the_name_filter_is_a_regex() {
        let mut state = state();
        state.filters.name = r"^Film\.202[45]".to_owned();
        let names: Vec<&str> = state
            .filtered()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["Film.2024.mkv", "Film.2025.mkv"]);

        state.filters.name = "film".to_owned();
        assert!(state.filtered().is_empty(), "the default is case sensitive");
    }

    #[test]
    fn the_tracker_filter_matches_any_announce_url() {
        let mut state = state();
        state.filters.tracker = "tracker-b".to_owned();
        let hashes: Vec<&str> = state
            .filtered()
            .iter()
            .map(|entry| entry.info_hash.as_str())
            .collect();
        assert_eq!(hashes, ["b", "c"]);
    }

    #[test]
    fn the_path_and_label_filters_are_exact() {
        let mut state = state();
        state.filters.save_path = Some("/mnt/filmek".to_owned());
        assert_eq!(state.filtered().len(), 2);
        state.filters.label = Some("movies".to_owned());
        assert!(
            state.filtered().is_empty(),
            "no candidate carries the label"
        );
    }

    #[test]
    fn only_missing_hides_what_the_daemon_has() {
        let mut state = state();
        state.entries[0].already_present = true;
        state.filters.only_missing = true;
        let hashes: Vec<&str> = state
            .filtered()
            .iter()
            .map(|entry| entry.info_hash.as_str())
            .collect();
        assert_eq!(hashes, ["b", "c"]);
    }

    #[test]
    fn an_invalid_pattern_is_an_error_and_hides_the_list() {
        let mut state = state();
        state.filters.name = "(".to_owned();
        assert!(state.filters.compile().is_err());
        assert!(state.filtered().is_empty(), "nothing is claimed to match");
    }

    #[test]
    fn selecting_filtered_only_takes_the_visible_matches() {
        let mut state = state();
        state.filters.tracker = "tracker-a".to_owned();
        state.select_filtered();
        assert_eq!(state.selection(), ["a"]);

        state.filters.tracker.clear();
        state.select_filtered();
        assert_eq!(state.selection(), ["a", "b", "c"]);
    }

    #[test]
    fn a_refresh_drops_selected_hashes_that_disappeared() {
        let mut state = state();
        state.select_filtered();
        state.set_preview(
            vec![entry(
                "Film.2024.mkv",
                "a",
                "/mnt/filmek",
                "http://tracker-a/announce",
            )],
            Vec::new(),
            Vec::new(),
            0,
        );
        assert_eq!(state.selection(), ["a"]);
    }

    #[test]
    fn the_window_follows_the_filtered_list() {
        let mut state = ImportState::new();
        let entries: Vec<ImportTorrentEntry> = (0..250)
            .map(|index| {
                entry(
                    &format!("torrent-{index:03}"),
                    &format!("{index:040x}"),
                    "/mnt/x",
                    "http://t/announce",
                )
            })
            .collect();
        state.set_preview(entries, Vec::new(), Vec::new(), 0);
        assert_eq!(state.window().0, 0);
        assert_eq!(state.window().1.len(), PAGE);

        state.offset = 100;
        let (start, rows) = state.window();
        assert_eq!(start, 100);
        assert_eq!(rows.len(), PAGE);

        state.offset = 200;
        assert_eq!(state.window().1.len(), 50, "the last window is partial");

        state.filters.name = r"torrent-2".to_owned();
        assert_eq!(state.filtered().len(), 50, "torrent-200..249");
        let (start, rows) = state.window();
        assert_eq!(start, 0, "the offset snaps back into the filtered list");
        assert_eq!(rows.len(), 50);
    }
}
