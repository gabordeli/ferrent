//! Add-path resolution: explicit path, then the label's staging folder, then
//! the session download directory.

use std::path::{Path, PathBuf};

use crate::config::{LowAction, ResolvedConfig};

use super::{disk_stat, resolve_staging};

/// Where an add writes its payload.
///
/// An explicit path always wins; a label without one goes to the staging
/// folder of the label's filesystem; everything else uses the session
/// download directory.
pub fn download_path(
    config: &ResolvedConfig,
    explicit: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let explicit = explicit.trim();
    if !explicit.is_empty() {
        return Ok(crate::rpc::resolve_save_path(
            &config.download_dir,
            explicit,
        ));
    }
    let label = label.trim();
    if label.is_empty() {
        return Ok(config.download_dir.clone());
    }
    match resolve_staging(&config.diskguard.zone, label) {
        Ok(path) => Ok(path),
        // A label without a zone yet keeps the old behaviour.
        Err(super::ZoneError::UnknownLabel(_)) => Ok(config.download_dir.clone()),
        Err(error) => Err(error.to_string()),
    }
}

/// Result of the free-space gate of an add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceGate {
    Ok,
    /// The add may proceed, but the caller should tell the user.
    Low(String),
    /// The add must not start.
    Reject(String),
}

/// Checks the resolved download path against the disk guard floors.
///
/// `total_bytes` is the payload size when known (zero for a magnet without
/// metadata); the optional preallocate gate needs it to refuse an add that
/// cannot fit.
pub fn space_gate(config: &ResolvedConfig, path: &Path, total_bytes: u64) -> SpaceGate {
    let minimum = config.diskguard.min_free_bytes;
    let preallocate = config.diskguard.require_space_for_torrent && total_bytes > 0;
    if minimum == 0 && !preallocate {
        return SpaceGate::Ok;
    }
    let free = disk_stat(path).map_or(0, |stat| stat.free);
    if preallocate {
        let needed = total_bytes.saturating_add(config.diskguard.margin_bytes);
        if free < needed {
            return SpaceGate::Reject(format!(
                "{} has {free} bytes free, {needed} required for the payload plus margin",
                path.display()
            ));
        }
    }
    if minimum == 0 || free >= minimum {
        return SpaceGate::Ok;
    }
    let message = format!(
        "{} has {free} bytes free, {minimum} required",
        path.display()
    );
    match config.diskguard.on_low {
        LowAction::Reject => SpaceGate::Reject(message),
        LowAction::Notify => SpaceGate::Low(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FileConfig;

    fn config(toml: &str) -> ResolvedConfig {
        let file = FileConfig::parse(toml).expect("config");
        file.resolve(&std::collections::BTreeMap::new())
            .expect("resolve")
    }

    fn zones(library: &str, staging: &str) -> String {
        format!(
            r#"
download_dir = "/data/downloads"

[[diskguard.zone]]
path = "{library}"
label = "Filmek"

[[diskguard.zone]]
path = "{staging}"
staging = true
"#
        )
    }

    #[test]
    fn an_explicit_path_wins_and_relative_paths_anchor_to_the_download_dir() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dp");
        std::fs::create_dir_all(&base).expect("base");
        let library = base.join("Filmek");
        let staging = &base;
        std::fs::create_dir_all(&library).expect("library");
        let config = config(&zones(
            &library.display().to_string(),
            &staging.display().to_string(),
        ));
        assert_eq!(
            download_path(&config, "custom", "Filmek").expect("explicit"),
            PathBuf::from("/data/downloads/custom")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_label_uses_the_staging_folder_of_its_disk() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dp2");
        std::fs::create_dir_all(&base).expect("base");
        let library = base.join("Filmek");
        std::fs::create_dir_all(&library).expect("library");
        let config = config(&zones(
            &library.display().to_string(),
            &base.display().to_string(),
        ));
        assert_eq!(download_path(&config, "", "filmek").expect("staging"), base);
        assert_eq!(
            download_path(&config, "  ", "").expect("session"),
            PathBuf::from("/data/downloads")
        );
        assert_eq!(
            download_path(&config, "", "Nope").expect("label without a zone"),
            PathBuf::from("/data/downloads")
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_space_gate_rejects_or_warns() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"));
        let reject = config(&format!("[diskguard]\nmin_free_bytes = {}\n", u64::MAX / 2));
        assert!(matches!(space_gate(&reject, path, 0), SpaceGate::Reject(_)));

        let notify = config(&format!(
            "[diskguard]\nmin_free_bytes = {}\non_low = \"notify\"\n",
            u64::MAX / 2
        ));
        assert!(matches!(space_gate(&notify, path, 0), SpaceGate::Low(_)));

        let off = config("");
        assert_eq!(space_gate(&off, path, 0), SpaceGate::Ok);

        let roomy = config("[diskguard]\nmin_free_bytes = 1\n");
        assert_eq!(space_gate(&roomy, path, 0), SpaceGate::Ok);

        // The preallocate gate needs the payload size and ignores unknown ones.
        let payload =
            "[diskguard]\nrequire_space_for_torrent = true\nmargin_bytes = 0\n".to_owned();
        let gate = config(&payload);
        assert_eq!(space_gate(&gate, path, 0), SpaceGate::Ok, "unknown size");
        assert_eq!(space_gate(&gate, path, 1024), SpaceGate::Ok);
        let huge = u64::MAX / 2;
        match space_gate(&gate, path, huge) {
            SpaceGate::Reject(message) => assert!(message.contains("required")),
            other => panic!("expected a rejection, got {other:?}"),
        }
    }
}
