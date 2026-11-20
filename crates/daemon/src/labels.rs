//! Labels: definitions, assignments, effective limits and move-on-complete.
//!
//! Definitions and assignments are runtime state in redb. Global defaults stay
//! in the config service; a label only overrides what it sets.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferrent_engine::{TorrentId, TorrentLimits};
use serde::{Deserialize, Serialize};

use crate::store::{StateStore, StoreError, StoreOp};

const LABEL_PREFIX: &str = "label:";

/// A named policy. Zero limits inherit the manual override or the session
/// default; an empty move path means "do not move on completion".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub move_completed_path: Option<PathBuf>,
    pub download_rate_limit: u64,
    pub upload_rate_limit: u64,
    pub connections_limit: u32,
    pub upload_slots_limit: u32,
    pub match_trackers: Vec<String>,
    pub match_name: Option<String>,
}

impl Label {
    /// Lowercase, URL-safe label name.
    pub fn normalize_name(name: &str) -> String {
        name.trim().to_ascii_lowercase()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.name.len() > 64 {
            return Err("label name must be 1..=64 characters".to_owned());
        }
        if !self
            .name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err("label name may contain letters, digits, '.', '_' and '-'".to_owned());
        }
        if !self.color.is_empty() {
            let hex = self.color.strip_prefix('#').unwrap_or("");
            if !(hex.len() == 6 || hex.len() == 8)
                || !hex.chars().all(|character| character.is_ascii_hexdigit())
            {
                return Err("label color must be #RRGGBB or #RRGGBBAA".to_owned());
            }
        }
        if self
            .move_completed_path
            .as_deref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err("move_completed_path must not be empty".to_owned());
        }
        if let Some(pattern) = &self.match_name {
            regex::Regex::new(pattern)
                .map_err(|error| format!("invalid label name regex: {error}"))?;
        }
        if self
            .match_trackers
            .iter()
            .any(|tracker| tracker.trim().is_empty())
        {
            return Err("match_trackers must not contain empty strings".to_owned());
        }
        Ok(())
    }

    #[must_use]
    pub fn limits(&self) -> TorrentLimits {
        TorrentLimits {
            download_rate_limit: self.download_rate_limit,
            upload_rate_limit: self.upload_rate_limit,
            connections_limit: self.connections_limit,
            upload_slots_limit: self.upload_slots_limit,
        }
    }
}

/// In-memory label definitions, assignments and manual overrides.
#[derive(Debug, Default)]
pub struct LabelState {
    labels: BTreeMap<String, Label>,
    assignments: BTreeMap<TorrentId, String>,
    overrides: BTreeMap<TorrentId, TorrentLimits>,
}

impl LabelState {
    /// Loads definitions from redb and assignments from the persisted records.
    /// Assignments whose label no longer exists are dropped.
    pub fn load(
        store: &dyn StateStore,
        torrents: &[crate::persist::PersistedTorrent],
    ) -> Result<Self, StoreError> {
        let labels = load_labels(store)?;
        let mut state = Self {
            labels,
            ..Self::default()
        };
        for torrent in torrents {
            if let Some(label) = &torrent.label {
                if state.labels.contains_key(label) {
                    state.assignments.insert(torrent.id, label.clone());
                }
            }
        }
        Ok(state)
    }

    #[must_use]
    pub fn list(&self) -> Vec<Label> {
        self.labels.values().cloned().collect()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Label> {
        self.labels.get(name)
    }

    #[must_use]
    pub fn has_matchers(&self) -> bool {
        self.labels
            .values()
            .any(|label| label.match_name.is_some() || !label.match_trackers.is_empty())
    }

    pub fn upsert(&mut self, label: Label) -> Result<(), String> {
        label.validate()?;
        self.labels.insert(label.name.clone(), label);
        Ok(())
    }

    /// Removes a label and every assignment to it; returns the affected torrents.
    pub fn remove(&mut self, name: &str) -> Vec<TorrentId> {
        self.labels.remove(name);
        let affected: Vec<TorrentId> = self
            .assignments
            .iter()
            .filter(|(_, label)| label.as_str() == name)
            .map(|(id, _)| *id)
            .collect();
        for id in &affected {
            self.assignments.remove(id);
        }
        affected
    }

    pub fn assign(&mut self, id: TorrentId, label: Option<String>) -> Result<(), String> {
        if let Some(name) = &label {
            if !self.labels.contains_key(name) {
                return Err(format!("label {name:?} does not exist"));
            }
        }
        if let Some(name) = label {
            self.assignments.insert(id, name);
        } else {
            self.assignments.remove(&id);
        }
        Ok(())
    }

    #[must_use]
    pub fn label(&self, id: TorrentId) -> Option<&str> {
        self.assignments.get(&id).map(String::as_str)
    }

    #[must_use]
    pub fn assignments(&self) -> BTreeMap<TorrentId, String> {
        self.assignments.clone()
    }

    #[must_use]
    pub fn counts(&self) -> BTreeMap<String, u32> {
        let mut counts: BTreeMap<String, u32> =
            self.labels.keys().map(|name| (name.clone(), 0)).collect();
        for label in self.assignments.values() {
            if let Some(count) = counts.get_mut(label) {
                *count += 1;
            }
        }
        counts
    }

    pub fn set_override(&mut self, id: TorrentId, limits: Option<TorrentLimits>) {
        if let Some(limits) = limits {
            self.overrides.insert(id, limits);
        } else {
            self.overrides.remove(&id);
        }
    }

    /// Manual override > label policy > zero (unlimited).
    #[must_use]
    pub fn limits(&self, id: TorrentId) -> TorrentLimits {
        if let Some(limits) = self.overrides.get(&id) {
            return *limits;
        }
        self.label(id)
            .and_then(|name| self.labels.get(name))
            .map(Label::limits)
            .unwrap_or_default()
    }

    /// Move target from the assigned label, if any.
    #[must_use]
    pub fn move_target(&self, id: TorrentId) -> Option<PathBuf> {
        self.label(id)
            .and_then(|name| self.labels.get(name))
            .and_then(|label| label.move_completed_path.clone())
    }

    /// A new or existing torrent without a label. Name regexes win over
    /// tracker substrings; labels are checked in name order.
    #[must_use]
    pub fn match_label(&self, name: &str, trackers: &[String]) -> Option<String> {
        for label in self.labels.values() {
            if let Some(pattern) = &label.match_name {
                if regex::Regex::new(pattern).is_ok_and(|regex| regex.is_match(name)) {
                    return Some(label.name.clone());
                }
            }
        }
        for label in self.labels.values() {
            if label.match_trackers.iter().any(|needle| {
                trackers
                    .iter()
                    .any(|url| url.to_lowercase().contains(&needle.to_lowercase()))
            }) {
                return Some(label.name.clone());
            }
        }
        None
    }

    pub fn remove_torrent(&mut self, id: TorrentId) {
        self.assignments.remove(&id);
        self.overrides.remove(&id);
    }
}

/// Loads every label definition from redb.
pub fn load_labels(store: &dyn StateStore) -> Result<BTreeMap<String, Label>, StoreError> {
    let entries = store.list(LABEL_PREFIX)?;
    let mut labels = BTreeMap::new();
    for (key, value) in entries {
        let name = key.strip_prefix(LABEL_PREFIX).unwrap_or(&key).to_owned();
        let text = String::from_utf8(value)
            .map_err(|error| StoreError::Backend(format!("invalid label {name}: {error}")))?;
        let label: Label = toml::from_str(&text)
            .map_err(|error| StoreError::Backend(format!("invalid label {name}: {error}")))?;
        labels.insert(label.name.clone(), label);
    }
    Ok(labels)
}

/// Saves one label definition.
pub fn save_label(store: &mut dyn StateStore, label: &Label) -> Result<(), StoreError> {
    let text = toml::to_string(label)
        .map_err(|error| StoreError::Backend(format!("cannot serialize label: {error}")))?;
    store.batch(vec![StoreOp::Put {
        key: format!("{LABEL_PREFIX}{}", label.name),
        value: text.into_bytes(),
    }])
}

/// Deletes one label definition (assignments are cleared separately).
pub fn delete_label(store: &mut dyn StateStore, name: &str) -> Result<(), StoreError> {
    store.batch(vec![StoreOp::Delete {
        key: format!("{LABEL_PREFIX}{name}"),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn label(name: &str, move_path: Option<&str>) -> Label {
        Label {
            name: name.to_owned(),
            color: "#1677F2".to_owned(),
            move_completed_path: move_path.map(PathBuf::from),
            download_rate_limit: 1024,
            upload_rate_limit: 512,
            connections_limit: 20,
            upload_slots_limit: 4,
            match_trackers: vec!["tracker.example".to_owned()],
            match_name: Some("^Movie".to_owned()),
        }
    }

    #[test]
    fn manual_override_wins_and_survives_label_removal() {
        let mut state = LabelState::default();
        state.upsert(label("movies", None)).expect("label");
        state.assign(1, Some("movies".to_owned())).expect("assign");
        let label_limits = state.limits(1);
        assert_eq!(label_limits.download_rate_limit, 1024);

        let manual = TorrentLimits {
            download_rate_limit: 2048,
            ..TorrentLimits::default()
        };
        state.set_override(1, Some(manual));
        assert_eq!(state.limits(1), manual);

        state.assign(1, None).expect("unassign");
        assert_eq!(state.limits(1), manual, "manual override must stay");
    }

    #[test]
    fn deleting_a_label_clears_assignments() {
        let mut state = LabelState::default();
        state.upsert(label("movies", None)).expect("label");
        state.assign(1, Some("movies".to_owned())).expect("assign");
        state.assign(2, Some("movies".to_owned())).expect("assign");
        assert_eq!(state.remove("movies"), vec![1, 2]);
        assert!(state.label(1).is_none());
        assert!(state.label(2).is_none());
    }

    #[test]
    fn name_regex_wins_over_tracker_match() {
        let mut state = LabelState::default();
        state
            .upsert(Label {
                name: "tracker".to_owned(),
                match_trackers: vec!["tracker.example".to_owned()],
                ..Label::default()
            })
            .expect("tracker label");
        state
            .upsert(Label {
                name: "name".to_owned(),
                match_name: Some("^Movie".to_owned()),
                ..Label::default()
            })
            .expect("name label");

        assert_eq!(
            state.match_label(
                "Movie Night",
                &["https://tracker.example/announce".to_owned()]
            ),
            Some("name".to_owned())
        );
        assert_eq!(
            state.match_label("Other", &["https://tracker.example/announce".to_owned()]),
            Some("tracker".to_owned())
        );
    }

    #[test]
    fn definitions_roundtrip_through_redb() {
        let mut store = MemoryStore::new();
        save_label(&mut store, &label("movies", Some("/mnt/media"))).expect("save");
        let labels = load_labels(&store).expect("load");
        assert_eq!(labels.len(), 1);
        assert_eq!(
            labels["movies"].move_completed_path,
            Some(PathBuf::from("/mnt/media"))
        );

        delete_label(&mut store, "movies").expect("delete");
        assert!(load_labels(&store).expect("reload").is_empty());
    }

    #[test]
    fn move_target_and_limits_resolve() {
        let mut state = LabelState::default();
        state
            .upsert(label("movies", Some("/mnt/media")))
            .expect("label");
        state.assign(7, Some("movies".to_owned())).expect("assign");
        assert_eq!(state.move_target(7), Some(PathBuf::from("/mnt/media")));
        assert_eq!(state.limits(7).upload_rate_limit, 512);
    }
}
