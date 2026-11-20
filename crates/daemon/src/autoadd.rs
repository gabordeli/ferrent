//! AutoAdd: watch directories that feed `.torrent` and `.magnet` files into
//! the engine, plus the persistence needed to survive restarts.
//!
//! The scan is a 1 s reconcile poll with a size/mtime stability check: a file
//! is only fed to the engine once two consecutive polls see the same length and
//! modification time, which covers partially copied torrent files. The 1 s poll
//! also meets the plugin port's 2 s pickup target without an inotify dependency.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::labels::LabelState;
use crate::store::StateStore;

/// A file that keeps failing for this many scans is renamed to `.invalid`.
pub const MAX_ATTEMPTS: u32 = 3;

/// Store key prefix for the per-file failure counter.
pub const ATTEMPT_PREFIX: &str = "autoadd:attempts:";

/// What happens to a file after it was accepted by the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnSuccess {
    /// Delete the source file.
    #[default]
    Delete,
    /// Leave the source file in place.
    Keep,
    /// Move the source file into [`WatchDir::move_to`].
    Move,
}

/// One configured watch directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WatchDir {
    pub path: PathBuf,
    pub enabled: bool,
    /// Overrides the session download directory for torrents from this folder.
    pub save_path: Option<PathBuf>,
    /// Label assigned to every torrent from this folder.
    pub label: Option<String>,
    pub add_paused: bool,
    pub on_success: OnSuccess,
    /// Destination for [`OnSuccess::Move`].
    pub move_to: Option<PathBuf>,
    /// Keep files in the watch directory while the save path has less free
    /// space than this; `0` disables the gate.
    pub min_free_bytes: u64,
}

impl Default for WatchDir {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            enabled: true,
            save_path: None,
            label: None,
            add_paused: false,
            on_success: OnSuccess::Delete,
            move_to: None,
            min_free_bytes: 0,
        }
    }
}

impl WatchDir {
    /// Checks the watch directory definition before it reaches the scanner.
    pub fn validate(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Err("watch path must not be empty".to_owned());
        }
        if self.on_success == OnSuccess::Move {
            let Some(move_to) = &self.move_to else {
                return Err("on_success = \"move\" needs a move_to path".to_owned());
            };
            if move_to.as_os_str().is_empty() {
                return Err("move_to must not be empty".to_owned());
            }
            if move_to == &self.path {
                return Err("move_to must differ from the watch path".to_owned());
            }
        }
        if let Some(label) = &self.label {
            if label.trim().is_empty() {
                return Err("label must not be empty".to_owned());
            }
        }
        Ok(())
    }

    /// Resolves the save path for a torrent from this folder.
    #[must_use]
    pub fn save_path(&self, session_download_dir: &Path) -> PathBuf {
        self.save_path
            .clone()
            .unwrap_or_else(|| session_download_dir.to_owned())
    }

    /// Whether a file name matches the configured extensions.
    #[must_use]
    pub fn accepts(path: &Path) -> bool {
        matches!(extension(path).as_deref(), Some("torrent" | "magnet"))
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

/// The engine source parsed from a watch file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Torrent { metainfo: Vec<u8>, summary: Summary },
    Magnet { links: Vec<String> },
}

impl Source {
    /// Display name suitable for persistence, labels and the torrent list.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Torrent { summary, .. } => summary.name.clone(),
            Self::Magnet { links } => links
                .first()
                .map(|link| magnet_name(link))
                .unwrap_or_else(|| "magnet".to_owned()),
        }
    }

    /// Known size in bytes (`0` when the magnet metadata is not here yet).
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        match self {
            Self::Torrent { summary, .. } => summary.total_bytes,
            Self::Magnet { .. } => 0,
        }
    }
}

/// Reads a watch file into one or more engine sources.
pub fn read_source(path: &Path) -> Result<Vec<Source>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    match extension(path).as_deref() {
        Some("torrent") => {
            let summary = Summary::parse(&bytes)?;
            Ok(vec![Source::Torrent {
                metainfo: bytes,
                summary,
            }])
        }
        Some("magnet") => {
            let text = String::from_utf8_lossy(&bytes);
            let links = magnet_links(&text);
            if links.is_empty() {
                return Err("no magnet link in file".to_owned());
            }
            Ok(vec![Source::Magnet { links }])
        }
        _ => Err("unsupported extension".to_owned()),
    }
}

/// Splits a magnet file into its links, one per line.
#[must_use]
pub fn magnet_links(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("magnet:?"))
        .map(str::to_owned)
        .collect()
}

/// Extracts the display name (`dn`) from a magnet link.
#[must_use]
pub fn magnet_name(link: &str) -> String {
    let query = link.split_once('?').map_or("", |(_, query)| query);
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("dn=") {
            return percent_decode(value);
        }
    }
    let infohash = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("xt=urn:btih:"))
        .unwrap_or_default();
    if infohash.is_empty() {
        "magnet".to_owned()
    } else {
        infohash.chars().take(12).collect()
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Name and total size read from a `.torrent` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub name: String,
    pub total_bytes: u64,
}

impl Summary {
    /// Parses the `info` dictionary of a metainfo file.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value = bencode(bytes)?;
        let Value::Dict(root) = value else {
            return Err("metainfo is not a dictionary".to_owned());
        };
        let info = root
            .into_iter()
            .find(|(key, _)| *key == b"info")
            .map(|(_, value)| value)
            .ok_or_else(|| "metainfo has no info dictionary".to_owned())?;
        let Value::Dict(info) = info else {
            return Err("metainfo info is not a dictionary".to_owned());
        };
        let mut name = None;
        let mut length = 0_u64;
        let mut files = 0_u64;
        for (key, value) in info {
            match key {
                b"name" => {
                    if let Value::Bytes(bytes) = value {
                        name = Some(String::from_utf8_lossy(bytes).into_owned());
                    }
                }
                b"length" => length = non_negative(value),
                b"files" => files = file_list_length(value),
                _ => {}
            }
        }
        let name = name.ok_or_else(|| "metainfo info has no name".to_owned())?;
        if name.trim().is_empty() {
            return Err("metainfo name is empty".to_owned());
        }
        Ok(Self {
            name,
            total_bytes: length.saturating_add(files),
        })
    }
}

fn non_negative(value: Value<'_>) -> u64 {
    match value {
        Value::Int(number) if number > 0 => number.unsigned_abs(),
        _ => 0,
    }
}

fn file_list_length(value: Value<'_>) -> u64 {
    let Value::List(entries) = value else {
        return 0;
    };
    entries
        .into_iter()
        .map(|entry| {
            let Value::Dict(fields) = entry else {
                return 0;
            };
            fields
                .into_iter()
                .find(|(key, _)| *key == b"length")
                .map_or(0, |(_, value)| non_negative(value))
        })
        .sum()
}

/// A minimal bencode reader: only what a metainfo summary needs.
enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

fn bencode(bytes: &[u8]) -> Result<Value<'_>, String> {
    let mut cursor = 0;
    let value = parse_value(bytes, &mut cursor)?;
    if cursor != bytes.len() {
        return Err("trailing bytes after the metainfo dictionary".to_owned());
    }
    Ok(value)
}

fn parse_value<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<Value<'a>, String> {
    let Some(&byte) = bytes.get(*cursor) else {
        return Err("unexpected end of metainfo".to_owned());
    };
    match byte {
        b'i' => {
            *cursor += 1;
            let end = bytes[*cursor..]
                .iter()
                .position(|&byte| byte == b'e')
                .map(|offset| *cursor + offset)
                .ok_or_else(|| "unterminated integer".to_owned())?;
            let text = std::str::from_utf8(&bytes[*cursor..end])
                .map_err(|_| "integer is not UTF-8".to_owned())?;
            let number: i64 = text.parse().map_err(|_| "invalid integer".to_owned())?;
            *cursor = end + 1;
            Ok(Value::Int(number))
        }
        b'l' => {
            *cursor += 1;
            let mut entries = Vec::new();
            while bytes.get(*cursor) != Some(&b'e') {
                entries.push(parse_value(bytes, cursor)?);
            }
            *cursor += 1;
            Ok(Value::List(entries))
        }
        b'd' => {
            *cursor += 1;
            let mut entries = Vec::new();
            while bytes.get(*cursor) != Some(&b'e') {
                let Value::Bytes(key) = parse_value(bytes, cursor)? else {
                    return Err("dictionary key is not a string".to_owned());
                };
                entries.push((key, parse_value(bytes, cursor)?));
            }
            *cursor += 1;
            Ok(Value::Dict(entries))
        }
        b'0'..=b'9' => {
            let colon = bytes[*cursor..]
                .iter()
                .position(|&byte| byte == b':')
                .map(|offset| *cursor + offset)
                .ok_or_else(|| "unterminated string length".to_owned())?;
            let text = std::str::from_utf8(&bytes[*cursor..colon])
                .map_err(|_| "string length is not UTF-8".to_owned())?;
            let length: usize = text
                .parse()
                .map_err(|_| "invalid string length".to_owned())?;
            let start = colon + 1;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| "string runs past the metainfo".to_owned())?;
            *cursor = end;
            Ok(Value::Bytes(&bytes[start..end]))
        }
        other => Err(format!("unexpected bencode byte {:?}", other as char)),
    }
}

/// A stable key for one watch file inside the state store.
#[must_use]
pub fn attempt_key(path: &Path) -> String {
    format!("{ATTEMPT_PREFIX}{}", path.to_string_lossy())
}

/// Reads the failure counter of one watch file.
#[must_use]
pub fn attempts(store: &dyn StateStore, path: &Path) -> u32 {
    store
        .get(&attempt_key(path))
        .ok()
        .flatten()
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

/// Increments the failure counter and returns the new value.
pub fn record_failure(store: &mut dyn StateStore, path: &Path) -> Result<u32, String> {
    let next = attempts(store, path).saturating_add(1);
    store
        .put(&attempt_key(path), &next.to_le_bytes())
        .map_err(|error| error.to_string())?;
    Ok(next)
}

/// Forgets the failure counter of one watch file.
pub fn clear_attempts(store: &mut dyn StateStore, path: &Path) {
    let _ = store.delete(&attempt_key(path));
}

/// Marks a file that exhausted its attempts. Returns the new path.
pub fn mark_invalid(path: &Path) -> Result<PathBuf, String> {
    let invalid = path.with_extension(format!(
        "{}.invalid",
        extension(path).unwrap_or_else(|| "torrent".to_owned())
    ));
    let target = unique_path(&invalid);
    std::fs::rename(path, &target)
        .map_err(|error| format!("rename {}: {error}", path.display()))?;
    Ok(target)
}

/// Applies the configured success action; returns the resulting path (`None`
/// when the file was deleted).
pub fn deliver(
    path: &Path,
    on_success: OnSuccess,
    move_to: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    match on_success {
        OnSuccess::Keep => Ok(Some(path.to_owned())),
        OnSuccess::Delete => {
            std::fs::remove_file(path)
                .map_err(|error| format!("remove {}: {error}", path.display()))?;
            Ok(None)
        }
        OnSuccess::Move => {
            let Some(directory) = move_to else {
                return Err("move action without a destination".to_owned());
            };
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("create {}: {error}", directory.display()))?;
            let name = path
                .file_name()
                .ok_or_else(|| "watch file has no name".to_owned())?;
            let target = unique_path(&directory.join(name));
            std::fs::rename(path, &target)
                .map_err(|error| format!("move {}: {error}", path.display()))?;
            Ok(Some(target))
        }
    }
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_owned();
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file");
    let extension = path.extension().and_then(|extension| extension.to_str());
    for index in 1..10_000 {
        let name = match extension {
            Some(extension) => format!("{stem}.{index}.{extension}"),
            None => format!("{stem}.{index}"),
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_owned()
}

/// One watch file seen by the scanner, with the stability fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl Fingerprint {
    /// Reads the fingerprint of a file.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata =
            std::fs::metadata(path).map_err(|error| format!("stat {}: {error}", path.display()))?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Candidate files of one watch directory, sorted by name.
pub fn candidates(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && WatchDir::accepts(path))
        .collect();
    files.sort();
    Ok(files)
}

/// Per-directory scanner counters surfaced to the GUI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirStatus {
    pub pending: u32,
    pub added: u32,
    pub duplicates: u32,
    pub invalid: u32,
    pub deferred: u32,
    pub last_error: Option<String>,
    pub last_scan: Option<SystemTime>,
}

/// Scanner state shared between the scan loop and the RPC layer.
#[derive(Debug, Default)]
pub struct AutoAddStatus {
    dirs: HashMap<PathBuf, DirStatus>,
    fingerprints: HashMap<PathBuf, Fingerprint>,
}

impl AutoAddStatus {
    /// Status of every directory seen so far.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(PathBuf, DirStatus)> {
        let mut dirs: Vec<_> = self
            .dirs
            .iter()
            .map(|(path, status)| (path.clone(), status.clone()))
            .collect();
        dirs.sort_by(|left, right| left.0.cmp(&right.0));
        dirs
    }

    /// Status of one directory.
    #[must_use]
    pub fn dir(&self, path: &Path) -> Option<DirStatus> {
        self.dirs.get(path).cloned()
    }

    pub(crate) fn begin_scan(&mut self, directory: &Path) {
        let status = self.dirs.entry(directory.to_owned()).or_default();
        status.pending = 0;
        status.last_scan = Some(SystemTime::now());
        status.last_error = None;
    }

    pub(crate) fn end_scan(&mut self, directory: &Path, error: Option<String>) {
        let status = self.dirs.entry(directory.to_owned()).or_default();
        if let Some(error) = error {
            status.last_error = Some(error);
        }
    }

    pub(crate) fn note_result(&mut self, path: &Path, result: &FileResult) {
        let Some(directory) = path.parent() else {
            return;
        };
        let status = self.dirs.entry(directory.to_owned()).or_default();
        status.added = status.added.saturating_add(result.added);
        status.duplicates = status.duplicates.saturating_add(result.duplicates);
        match result.outcome {
            FileOutcome::Waiting => status.pending = status.pending.saturating_add(1),
            FileOutcome::Invalid => status.invalid = status.invalid.saturating_add(1),
            FileOutcome::Deferred => status.deferred = status.deferred.saturating_add(1),
            FileOutcome::Added | FileOutcome::Duplicate | FileOutcome::Failed => {}
        }
    }

    pub(crate) fn note_error(&mut self, path: &Path, error: String) {
        if let Some(directory) = path.parent() {
            self.dirs
                .entry(directory.to_owned())
                .or_default()
                .last_error = Some(error);
        }
    }

    /// Returns `true` when the file is stable enough to process this tick.
    pub(crate) fn stable(&mut self, path: &Path, fingerprint: Fingerprint) -> bool {
        match self.fingerprints.get(path) {
            Some(previous) if *previous == fingerprint => true,
            _ => {
                self.fingerprints.insert(path.to_owned(), fingerprint);
                false
            }
        }
    }

    pub(crate) fn forget(&mut self, path: &Path) {
        self.fingerprints.remove(path);
    }

    /// Forces the next scan to re-read every file.
    pub(crate) fn clear_fingerprints(&mut self) {
        self.fingerprints.clear();
    }

    /// Forces the next scan of one directory to re-read its files.
    pub(crate) fn clear_fingerprints_for(&mut self, directory: &Path) {
        self.fingerprints
            .retain(|path, _| path.parent() != Some(directory));
    }
}

/// Per-file scanner result: torrent counts plus the file outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileResult {
    pub outcome: FileOutcome,
    pub added: u32,
    pub duplicates: u32,
}

impl FileResult {
    #[must_use]
    pub fn outcome(outcome: FileOutcome) -> Self {
        Self {
            outcome,
            added: 0,
            duplicates: 0,
        }
    }

    #[must_use]
    pub fn waiting() -> Self {
        Self::outcome(FileOutcome::Waiting)
    }
}

/// What the scanner did with one file (for the counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    Waiting,
    Added,
    Duplicate,
    Invalid,
    Failed,
    Deferred,
}

/// Free bytes on the filesystem holding `path`, walking up to an existing
/// ancestor so a not-yet-created save directory still reports its mount.
#[must_use]
pub fn free_bytes(path: &Path) -> u64 {
    let mut candidate = path;
    loop {
        if let Ok(stat) = rustix::fs::statvfs(candidate) {
            return stat.f_bavail.saturating_mul(stat.f_frsize);
        }
        match candidate.parent() {
            Some(parent) => candidate = parent,
            None => return 0,
        }
    }
}

/// Resolves the label of a watch directory against the known definitions.
#[must_use]
pub fn label_exists(labels: &LabelState, label: &str) -> bool {
    labels.get(label).is_some()
}

/// Counters for one [`AppState::scan_once`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanTotals {
    pub added: u32,
    pub duplicates: u32,
    pub failed: u32,
    pub invalid: u32,
    pub waiting: u32,
    pub deferred: u32,
}

/// Result of one engine add from the AutoAdd path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddOutcome {
    pub id: u64,
    pub duplicate: bool,
    /// Non-fatal policy note (e.g. a configured label does not exist).
    pub warning: Option<String>,
}

/// Starts the 1 s reconcile loop.
pub fn spawn_autoadd(state: std::sync::Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            state.scan_once().await;
        }
    });
}

impl crate::rpc::AppState {
    /// Scans every enabled watch directory once.
    pub async fn scan_once(&self) -> ScanTotals {
        self.scan_once_inner(false).await
    }

    async fn scan_once_inner(&self, force: bool) -> ScanTotals {
        let Ok(config) = self.config.resolved() else {
            return ScanTotals::default();
        };
        let mut totals = ScanTotals::default();
        for watch in config.autoadd.iter().filter(|watch| watch.enabled) {
            self.scan_dir(watch, &self.download_dir, &mut totals, force)
                .await;
        }
        totals
    }

    /// Current scanner counters per watch directory.
    #[must_use]
    pub fn autoadd_status(&self) -> Vec<(PathBuf, DirStatus)> {
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .snapshot()
    }

    /// Forgets the stability fingerprints and scans immediately.
    pub async fn scan_now(&self) -> ScanTotals {
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .clear_fingerprints();
        self.scan_once_inner(true).await
    }

    /// Scans one watch directory immediately, if it is configured.
    pub async fn scan_watch_now(&self, path: &Path) -> Option<ScanTotals> {
        let config = self.config.resolved().ok()?;
        let watch = config
            .autoadd
            .iter()
            .find(|watch| watch.path == path)?
            .clone();
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .clear_fingerprints_for(path);
        let mut totals = ScanTotals::default();
        self.scan_dir(&watch, &self.download_dir, &mut totals, true)
            .await;
        Some(totals)
    }

    async fn scan_dir(
        &self,
        watch: &WatchDir,
        session_dir: &Path,
        totals: &mut ScanTotals,
        force: bool,
    ) {
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .begin_scan(&watch.path);
        let files = match candidates(&watch.path) {
            Ok(files) => files,
            Err(error) => {
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .end_scan(&watch.path, Some(error));
                return;
            }
        };
        for path in files {
            let fingerprint = match Fingerprint::read(&path) {
                Ok(fingerprint) => fingerprint,
                Err(error) => {
                    self.autoadd
                        .lock()
                        .expect("autoadd status mutex poisoned")
                        .note_error(&path, error);
                    continue;
                }
            };
            let stable = force
                || self
                    .autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .stable(&path, fingerprint);
            if !stable {
                totals.waiting = totals.waiting.saturating_add(1);
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .note_result(&path, &FileResult::waiting());
                continue;
            }
            let result = self.process_file(watch, &path, session_dir).await;
            {
                let mut status = self.autoadd.lock().expect("autoadd status mutex poisoned");
                status.note_result(&path, &result);
                if matches!(result.outcome, FileOutcome::Added | FileOutcome::Duplicate) {
                    status.forget(&path);
                }
            }
            totals.added = totals.added.saturating_add(result.added);
            totals.duplicates = totals.duplicates.saturating_add(result.duplicates);
            match result.outcome {
                FileOutcome::Invalid => totals.invalid = totals.invalid.saturating_add(1),
                FileOutcome::Failed => totals.failed = totals.failed.saturating_add(1),
                FileOutcome::Waiting => totals.waiting = totals.waiting.saturating_add(1),
                FileOutcome::Deferred => totals.deferred = totals.deferred.saturating_add(1),
                FileOutcome::Added | FileOutcome::Duplicate => {}
            }
        }
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .end_scan(&watch.path, None);
    }

    async fn process_file(&self, watch: &WatchDir, path: &Path, session_dir: &Path) -> FileResult {
        let sources = match read_source(path) {
            Ok(sources) => sources,
            Err(error) => return FileResult::outcome(self.fail(path, &error)),
        };
        let save_path = watch.save_path(session_dir);
        if watch.min_free_bytes > 0 {
            let free = free_bytes(&save_path);
            if free < watch.min_free_bytes {
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .note_error(
                        path,
                        format!(
                            "deferred: {free} bytes free on {}, {} required",
                            save_path.display(),
                            watch.min_free_bytes
                        ),
                    );
                return FileResult::outcome(FileOutcome::Deferred);
            }
        }
        let mut added = 0_u32;
        let mut duplicates = 0_u32;
        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for source in sources {
            let specs: Vec<(String, u64, ferrent_engine::TorrentSource)> = match source {
                Source::Torrent {
                    metainfo, summary, ..
                } => vec![(
                    summary.name,
                    summary.total_bytes,
                    ferrent_engine::TorrentSource::Metainfo(metainfo),
                )],
                Source::Magnet { links } => links
                    .into_iter()
                    .map(|link| {
                        (
                            magnet_name(&link),
                            0,
                            ferrent_engine::TorrentSource::Magnet(link),
                        )
                    })
                    .collect(),
            };
            for (name, total_bytes, engine_source) in specs {
                let spec = ferrent_engine::AddTorrent {
                    name,
                    total_bytes,
                    source: engine_source,
                    save_path: save_path.clone(),
                    resume_data: None,
                };
                match self
                    .add_source(spec, watch.add_paused, watch.label.clone())
                    .await
                {
                    Ok(outcome) => {
                        if outcome.duplicate {
                            duplicates = duplicates.saturating_add(1);
                        } else {
                            added = added.saturating_add(1);
                        }
                        warnings.extend(outcome.warning);
                    }
                    Err(error) => errors.push(error),
                }
            }
        }
        if added == 0 && duplicates == 0 {
            let message = if errors.is_empty() {
                "no torrent source".to_owned()
            } else {
                errors.join("; ")
            };
            return FileResult::outcome(self.fail(path, &message));
        }
        if !warnings.is_empty() {
            self.autoadd
                .lock()
                .expect("autoadd status mutex poisoned")
                .note_error(path, warnings.join("; "));
        }
        if !errors.is_empty() {
            self.autoadd
                .lock()
                .expect("autoadd status mutex poisoned")
                .note_error(path, errors.join("; "));
        }
        match deliver(path, watch.on_success, watch.move_to.as_deref()) {
            Ok(_) => {
                let mut store = self.store.lock().expect("state store mutex poisoned");
                clear_attempts(&mut **store, path);
                let _ = store.flush();
            }
            Err(error) => {
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .note_error(path, error);
            }
        }
        FileResult {
            outcome: if added > 0 {
                FileOutcome::Added
            } else {
                FileOutcome::Duplicate
            },
            added,
            duplicates,
        }
    }

    fn fail(&self, path: &Path, message: &str) -> FileOutcome {
        let attempts = {
            let mut store = self.store.lock().expect("state store mutex poisoned");
            let attempts = record_failure(&mut **store, path).unwrap_or(MAX_ATTEMPTS);
            if attempts >= MAX_ATTEMPTS {
                clear_attempts(&mut **store, path);
            }
            let _ = store.flush();
            attempts
        };
        self.autoadd
            .lock()
            .expect("autoadd status mutex poisoned")
            .note_error(path, message.to_owned());
        if attempts < MAX_ATTEMPTS {
            return FileOutcome::Failed;
        }
        match mark_invalid(path) {
            Ok(target) => {
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .note_error(path, format!("marked {}", target.display()));
                FileOutcome::Invalid
            }
            Err(error) => {
                self.autoadd
                    .lock()
                    .expect("autoadd status mutex poisoned")
                    .note_error(path, error);
                FileOutcome::Failed
            }
        }
    }

    /// Adds one source for AutoAdd: engine, persistence, label policy, pause.
    pub async fn add_source(
        &self,
        spec: ferrent_engine::AddTorrent,
        paused: bool,
        label: Option<String>,
    ) -> Result<AddOutcome, String> {
        let name = spec.name.clone();
        let save_path = spec.save_path.clone();
        let metainfo = match &spec.source {
            ferrent_engine::TorrentSource::Metainfo(bytes) => bytes.clone(),
            ferrent_engine::TorrentSource::Magnet(_) => Vec::new(),
        };
        let engine = std::sync::Arc::clone(&self.engine);
        let (id, duplicate, trackers) = tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            match engine.add_torrent(spec) {
                Ok(id) => {
                    let trackers = engine
                        .trackers(id)
                        .map(|items| items.into_iter().map(|tracker| tracker.url).collect())
                        .unwrap_or_default();
                    Ok((id, false, trackers))
                }
                Err(ferrent_engine::EngineError::AlreadyExists { id }) => {
                    Ok((id, true, Vec::new()))
                }
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error: ferrent_engine::EngineError| error.to_string())?;
        if duplicate {
            return Ok(AddOutcome {
                id,
                duplicate: true,
                warning: None,
            });
        }

        let persisted = crate::persist::PersistedTorrent {
            id,
            name: name.clone(),
            save_path,
            metainfo,
            resume_data: None,
            paused_by_user: paused,
            label: label.clone(),
        };
        let store = std::sync::Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().expect("state store mutex poisoned");
            crate::persist::save_torrent(&mut **store, &persisted)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("state store: {error}"))?;

        let mut warning = None;
        let target = match label {
            Some(label) => {
                if self
                    .labels
                    .lock()
                    .expect("label state poisoned")
                    .get(&label)
                    .is_some()
                {
                    Some(label)
                } else {
                    warning = Some(format!("label {label:?} is not defined"));
                    None
                }
            }
            None => self
                .labels
                .lock()
                .expect("label state poisoned")
                .match_label(&name, &trackers),
        };
        if let Some(label) = target {
            {
                let mut labels = self.labels.lock().expect("label state poisoned");
                labels
                    .assign(id, Some(label.clone()))
                    .map_err(|error| error.to_string())?;
            }
            let persisted_label = label.clone();
            let store = std::sync::Arc::clone(&self.store);
            let _ = tokio::task::spawn_blocking(move || {
                let mut store = store.lock().expect("state store mutex poisoned");
                crate::persist::save_torrent_label(&mut **store, id, Some(&persisted_label))
            })
            .await;
            let limits = self.labels.lock().expect("label state poisoned").limits(id);
            let engine = std::sync::Arc::clone(&self.engine);
            let _ = tokio::task::spawn_blocking(move || {
                let mut engine = engine.lock().expect("engine mutex poisoned");
                engine.set_torrent_limits(id, limits)
            })
            .await;
        }

        if paused {
            self.control
                .lock()
                .expect("pause control mutex poisoned")
                .set_user(id, true);
            let engine = std::sync::Arc::clone(&self.engine);
            let _ = tokio::task::spawn_blocking(move || {
                let mut engine = engine.lock().expect("engine mutex poisoned");
                engine.pause(id)
            })
            .await;
        }

        Ok(AddOutcome {
            id,
            duplicate: false,
            warning,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn torrent_bytes(name: &str, length: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"d4:infod6:lengthi");
        bytes.extend_from_slice(length.to_string().as_bytes());
        bytes.extend_from_slice(b"e4:name");
        bytes.extend_from_slice(name.len().to_string().as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(b"ee");
        bytes
    }

    #[test]
    fn summary_reads_the_info_dictionary() {
        let summary = Summary::parse(&torrent_bytes("movie.mkv", 1234)).expect("summary");
        assert_eq!(summary.name, "movie.mkv");
        assert_eq!(summary.total_bytes, 1234);
    }

    #[test]
    fn summary_sums_the_file_list() {
        let bytes =
            b"d4:infod5:filesld6:lengthi10e4:pathl1:aeed6:lengthi32e4:pathl1:beee4:name4:showee";
        let summary = Summary::parse(bytes).expect("summary");
        assert_eq!(summary.name, "show");
        assert_eq!(summary.total_bytes, 42);
    }

    #[test]
    fn broken_metainfo_is_rejected() {
        assert!(Summary::parse(b"").is_err());
        assert!(
            Summary::parse(b"d4:infoi1ee").is_err(),
            "info is not a dict"
        );
        assert!(Summary::parse(b"d4:infod4:name0:ee").is_err(), "empty name");
        assert!(Summary::parse(b"d4:info").is_err(), "truncated");
    }

    #[test]
    fn magnet_files_split_into_links() {
        let text = "\n# comment\nmagnet:?xt=urn:btih:abc&dn=First%20File\n\nmagnet:?xt=urn:btih:def&dn=Second\n";
        let links = magnet_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!(magnet_name(&links[0]), "First File");
        assert_eq!(magnet_name(&links[1]), "Second");
        assert_eq!(
            magnet_name("magnet:?xt=urn:btih:abcdef123456"),
            "abcdef123456"
        );
    }

    #[test]
    fn watch_dir_validation_covers_the_move_action() {
        let mut dir = WatchDir::default();
        assert!(dir.validate().is_err(), "empty path");
        dir.path = "/data/watch".into();
        assert!(dir.validate().is_ok());
        dir.on_success = OnSuccess::Move;
        assert!(dir.validate().is_err(), "missing destination");
        dir.move_to = Some("/data/watch".into());
        assert!(dir.validate().is_err(), "same as the watch path");
        dir.move_to = Some("/data/done".into());
        assert!(dir.validate().is_ok());
        dir.label = Some("  ".to_owned());
        assert!(dir.validate().is_err());
    }

    #[test]
    fn attempts_survive_in_the_store() {
        let mut store = MemoryStore::new();
        let path = Path::new("/data/watch/a.torrent");
        assert_eq!(attempts(&store, path), 0);
        assert_eq!(record_failure(&mut store, path).expect("count"), 1);
        assert_eq!(record_failure(&mut store, path).expect("count"), 2);
        assert_eq!(attempts(&store, path), 2);
        clear_attempts(&mut store, path);
        assert_eq!(attempts(&store, path), 0);
    }

    #[test]
    fn deliver_keeps_deletes_and_moves() {
        let base = std::env::temp_dir().join(format!("ferrent-autoadd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let watch = base.join("watch");
        let done = base.join("done");
        std::fs::create_dir_all(&watch).expect("watch dir");

        let keep = watch.join("keep.torrent");
        std::fs::write(&keep, b"d4:infoe").expect("write");
        assert_eq!(
            deliver(&keep, OnSuccess::Keep, None).expect("keep"),
            Some(keep.clone())
        );
        assert!(keep.exists());

        let delete = watch.join("delete.torrent");
        std::fs::write(&delete, b"d4:infoe").expect("write");
        assert_eq!(
            deliver(&delete, OnSuccess::Delete, None).expect("delete"),
            None
        );
        assert!(!delete.exists());

        let moved = watch.join("moved.torrent");
        std::fs::write(&moved, b"d4:infoe").expect("write");
        let target = deliver(&moved, OnSuccess::Move, Some(&done))
            .expect("move")
            .expect("target");
        assert_eq!(target, done.join("moved.torrent"));
        assert!(target.exists());
        assert!(!moved.exists());

        // A second file with the same name does not clobber the first one.
        std::fs::write(&moved, b"d4:infoe").expect("write");
        let second = deliver(&moved, OnSuccess::Move, Some(&done))
            .expect("move")
            .expect("target");
        assert_eq!(second, done.join("moved.1.torrent"));

        let invalid = watch.join("broken.torrent");
        std::fs::write(&invalid, b"nope").expect("write");
        let marked = mark_invalid(&invalid).expect("invalid");
        assert_eq!(marked, watch.join("broken.torrent.invalid"));
        assert!(marked.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_needs_two_equal_fingerprints() {
        let mut status = AutoAddStatus::default();
        let path = Path::new("/data/watch/a.torrent");
        let first = Fingerprint {
            len: 10,
            modified: None,
        };
        assert!(!status.stable(path, first));
        assert!(status.stable(path, first));
        assert!(!status.stable(
            path,
            Fingerprint {
                len: 11,
                modified: None
            }
        ));
        status.forget(path);
        assert!(!status.stable(path, first));
    }

    #[test]
    fn candidates_only_include_torrent_and_magnet_files() {
        let base =
            std::env::temp_dir().join(format!("ferrent-autoadd-cand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("dir");
        for name in ["b.torrent", "a.magnet", "note.txt", "c.TORRENT"] {
            std::fs::write(base.join(name), b"x").expect("write");
        }
        let files = candidates(&base).expect("candidates");
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.magnet", "b.torrent", "c.TORRENT"]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
