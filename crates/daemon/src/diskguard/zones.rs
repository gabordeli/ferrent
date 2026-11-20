//! Storage zones: labelled library folders and per-filesystem staging folders.
//!
//! A zone is a directory with a role, not a whole disk. A *library* zone is
//! the final home of exactly one label; a *staging* zone is the download
//! folder of one filesystem. Resolving a label therefore yields a staging
//! folder on the same filesystem as the label's library, which keeps the
//! completion move a rename instead of a cross-disk copy.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::labels::Label;

/// One configured storage zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Zone {
    /// Absolute directory of the zone.
    pub path: PathBuf,
    /// Label whose library this zone is; empty for a staging zone.
    pub label: Option<String>,
    /// Marks the download folder of the filesystem holding this zone.
    pub staging: bool,
    pub enabled: bool,
}

impl Default for Zone {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            label: None,
            staging: false,
            enabled: true,
        }
    }
}

impl Zone {
    /// Builds a library zone for a label.
    pub fn library(path: impl Into<PathBuf>, label: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            label: Some(label.into()),
            staging: false,
            enabled: true,
        }
    }

    /// Builds a staging zone.
    pub fn staging(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            label: None,
            staging: true,
            enabled: true,
        }
    }

    /// The normalised label name of a library zone.
    #[must_use]
    pub fn label_name(&self) -> Option<String> {
        self.label
            .as_deref()
            .map(Label::normalize_name)
            .filter(|name| !name.is_empty())
    }

    /// Checks a zone definition before it reaches the resolver.
    pub fn validate(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Err("zone path must not be empty".to_owned());
        }
        if !self.path.is_absolute() {
            return Err(format!(
                "zone path {} must be absolute",
                self.path.display()
            ));
        }
        match (&self.label, self.staging) {
            (Some(_), true) => Err("staging zone must not have a label".to_owned()),
            (None, false) => Err("library zone needs a label".to_owned()),
            (None, true) => Ok(()),
            (Some(label), false) => {
                let name = label.trim();
                if name.is_empty() {
                    return Err("zone label must not be empty".to_owned());
                }
                if name.chars().count() > 64
                    || !name.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "._-".contains(character)
                    })
                {
                    return Err(format!(
                        "label {label:?} may contain letters, digits, '.', '_' and '-'"
                    ));
                }
                Ok(())
            }
        }
    }
}

/// Checks a whole zone list: per-zone rules plus cross-zone duplicates.
pub fn validate_zones(zones: &[Zone]) -> Result<(), String> {
    let mut paths = std::collections::BTreeSet::new();
    let mut labels = std::collections::BTreeSet::new();
    for (index, zone) in zones.iter().enumerate() {
        zone.validate()
            .map_err(|message| format!("diskguard.zone[{index}]: {message}"))?;
        let path = zone.path.clone();
        if !paths.insert(path.clone()) {
            return Err(format!(
                "diskguard.zone[{index}]: duplicate zone path {}",
                path.display()
            ));
        }
        if let Some(label) = zone.label_name() {
            if !labels.insert(label.clone()) {
                return Err(format!(
                    "diskguard.zone[{index}]: label {label:?} already has a library zone"
                ));
            }
        }
    }
    Ok(())
}

/// Why a label could not be resolved to a staging folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneError {
    /// No enabled library zone uses this label.
    UnknownLabel(String),
    /// The library zone path does not exist (unmounted disk?).
    UnknownPath(PathBuf),
    /// The label's filesystem has no staging zone.
    MissingStaging { label: String, device: u64 },
}

impl fmt::Display for ZoneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLabel(label) => write!(formatter, "no library zone for label {label:?}"),
            Self::UnknownPath(path) => {
                write!(formatter, "zone path {} is not available", path.display())
            }
            Self::MissingStaging { label, device } => write!(
                formatter,
                "label {label:?} has no staging zone on its filesystem (device {device})"
            ),
        }
    }
}

impl std::error::Error for ZoneError {}

/// Finds the enabled library zone of a label.
#[must_use]
pub fn library_zone<'a>(zones: &'a [Zone], label: &str) -> Option<&'a Zone> {
    let wanted = Label::normalize_name(label);
    zones.iter().find(|zone| {
        zone.enabled && !zone.staging && zone.label_name().as_deref() == Some(wanted.as_str())
    })
}

/// Resolves a label to the staging folder of its filesystem.
///
/// `device` returns the filesystem id (`st_dev`) of a path; it is injected so
/// the resolver is testable without mounts.
pub fn resolve_staging_with(
    zones: &[Zone],
    label: &str,
    device: impl Fn(&Path) -> Option<u64>,
) -> Result<PathBuf, ZoneError> {
    let library =
        library_zone(zones, label).ok_or_else(|| ZoneError::UnknownLabel(label.to_owned()))?;
    let library_device =
        device(&library.path).ok_or_else(|| ZoneError::UnknownPath(library.path.clone()))?;
    zones
        .iter()
        .find(|zone| zone.enabled && zone.staging && device(&zone.path) == Some(library_device))
        .map(|zone| zone.path.clone())
        .ok_or_else(|| ZoneError::MissingStaging {
            label: label.to_owned(),
            device: library_device,
        })
}

/// Resolves a label against the real filesystem.
pub fn resolve_staging(zones: &[Zone], label: &str) -> Result<PathBuf, ZoneError> {
    resolve_staging_with(zones, label, super::storage::device_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_zones() -> Vec<Zone> {
        vec![
            Zone::library("/mnt/usbhdd2/Filmek", "Filmek"),
            Zone::library("/mnt/usbhdd2/Sorozatok", "Sorozatok"),
            Zone::library("/mnt/usbhdd3/for-seed", "seed"),
            Zone::staging("/mnt/usbhdd2/_progress"),
            Zone::staging("/mnt/usbhdd3/_progress"),
        ]
    }

    fn fake_device(path: &Path) -> Option<u64> {
        let text = path.to_string_lossy();
        if text.starts_with("/mnt/usbhdd2") {
            Some(2)
        } else if text.starts_with("/mnt/usbhdd3") {
            Some(3)
        } else {
            None
        }
    }

    #[test]
    fn a_label_resolves_to_the_staging_of_its_own_disk() {
        let resolved =
            resolve_staging_with(&sample_zones(), "filmek", fake_device).expect("resolve");
        assert_eq!(resolved, PathBuf::from("/mnt/usbhdd2/_progress"));

        let resolved = resolve_staging_with(&sample_zones(), "seed", fake_device).expect("resolve");
        assert_eq!(resolved, PathBuf::from("/mnt/usbhdd3/_progress"));
    }

    #[test]
    fn unknown_labels_and_missing_staging_are_distinct_errors() {
        assert_eq!(
            resolve_staging_with(&sample_zones(), "nope", fake_device),
            Err(ZoneError::UnknownLabel("nope".to_owned()))
        );

        let mut zones = sample_zones();
        zones.retain(|zone| zone.path != Path::new("/mnt/usbhdd3/_progress"));
        assert_eq!(
            resolve_staging_with(&zones, "seed", fake_device),
            Err(ZoneError::MissingStaging {
                label: "seed".to_owned(),
                device: 3
            })
        );

        assert_eq!(
            resolve_staging_with(&zones, "filmek", |_| None),
            Err(ZoneError::UnknownPath(PathBuf::from("/mnt/usbhdd2/Filmek")))
        );
    }

    #[test]
    fn disabled_zones_are_ignored() {
        let mut zones = sample_zones();
        zones[0].enabled = false;
        assert_eq!(
            resolve_staging_with(&zones, "Filmek", fake_device),
            Err(ZoneError::UnknownLabel("Filmek".to_owned()))
        );

        let mut zones = sample_zones();
        zones[3].enabled = false;
        assert!(matches!(
            resolve_staging_with(&zones, "Filmek", fake_device),
            Err(ZoneError::MissingStaging { .. })
        ));
    }

    #[test]
    fn zone_validation_covers_the_roles() {
        let mut zone = Zone::default();
        assert!(zone.validate().is_err(), "empty path");
        zone.path = PathBuf::from("relative");
        zone.staging = true;
        assert!(zone.validate().is_err(), "relative path");

        let mut library = Zone::library("/mnt/a/Filmek", "Filmek");
        assert!(library.validate().is_ok());
        library.label = None;
        assert!(library.validate().is_err(), "library needs a label");

        let mut staging = Zone::staging("/mnt/a/_progress");
        assert!(staging.validate().is_ok());
        staging.label = Some("Filmek".to_owned());
        assert!(staging.validate().is_err(), "staging has no label");

        assert!(Zone::library("/mnt/a/x", "").validate().is_err());
        assert!(Zone::library("/mnt/a/x", "with space").validate().is_err());
    }

    #[test]
    fn duplicate_paths_and_labels_are_rejected() {
        let mut list = sample_zones();
        assert!(validate_zones(&list).is_ok());
        list.push(Zone::library("/mnt/usbhdd2/Filmek", "Filmek2"));
        assert!(validate_zones(&list).is_err(), "duplicate path");

        let mut list = sample_zones();
        list.push(Zone::library("/mnt/usbhdd1/Filmek", "filmek"));
        let error = validate_zones(&list).expect_err("duplicate label");
        assert!(error.contains("filmek"), "{error}");
    }
}
