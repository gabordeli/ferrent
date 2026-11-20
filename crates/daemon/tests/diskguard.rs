#![cfg(unix)]

//! DiskGuard storage model: zones from the config, resolution against real
//! directories and the storage payload.

use std::path::{Path, PathBuf};

use ferrent_daemon::config::ConfigStore;
use ferrent_daemon::diskguard::{self, Zone, ZoneError};

fn base(tag: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("ferrent-diskguard-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("base");
    directory
}

fn zone_toml(library: &Path, staging: &Path) -> String {
    format!(
        r#"
[diskguard]
min_free_bytes = 1048576
poll_secs = 30

[[diskguard.zone]]
path = {library:?}
label = "Filmek"

[[diskguard.zone]]
path = {staging:?}
staging = true
"#,
        library = library.display().to_string(),
        staging = staging.display().to_string(),
    )
}

#[test]
fn a_label_resolves_to_the_staging_folder_of_its_filesystem() {
    let base = base("resolve");
    let library = base.join("Filmek");
    let staging = base.join("_progress");
    std::fs::create_dir_all(&library).expect("library");
    std::fs::create_dir_all(&staging).expect("staging");

    let store = ConfigStore::default();
    store
        .apply_toml(&zone_toml(&library, &staging), 0)
        .expect("config");
    let resolved = store.resolved().expect("resolved");
    let zones = &resolved.diskguard.zone;

    assert_eq!(
        diskguard::resolve_staging(zones, "filmek").expect("resolve"),
        staging
    );
    assert_eq!(
        diskguard::resolve_staging(zones, "Other"),
        Err(ZoneError::UnknownLabel("Other".to_owned()))
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_staging_folder_on_another_filesystem_is_rejected() {
    let base = base("cross-device");
    let library = base.join("Filmek");
    std::fs::create_dir_all(&library).expect("library");
    // The system temp directory is often a tmpfs; only assert the error when
    // the two paths really sit on different filesystems.
    std::fs::create_dir_all("/tmp").expect("tmp");
    let same = match (
        diskguard::device_id(&library),
        diskguard::device_id(Path::new("/tmp")),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    };

    let zones = vec![
        Zone::library(library.clone(), "Filmek"),
        Zone::staging("/tmp"),
    ];
    let result = diskguard::resolve_staging(&zones, "Filmek");
    if same {
        assert_eq!(result, Ok(PathBuf::from("/tmp")));
    } else {
        assert!(
            matches!(result, Err(ZoneError::MissingStaging { .. })),
            "{result:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn the_storage_payload_reports_the_disk_and_its_zones() {
    let base = base("payload");
    let library = base.join("Filmek");
    let staging = base.join("_progress");
    std::fs::create_dir_all(&library).expect("library");
    std::fs::create_dir_all(&staging).expect("staging");

    let store = ConfigStore::default();
    store
        .apply_toml(&zone_toml(&library, &staging), 0)
        .expect("config");
    let resolved = store.resolved().expect("resolved");
    let torrents = vec![(1, staging.join("one")), (2, library.join("two"))];
    let report = diskguard::snapshot(&resolved.diskguard.zone, &torrents, 0)
        .await
        .expect("snapshot");

    assert_eq!(report.disks.len(), 1, "{report:?}");
    let disk = &report.disks[0];
    assert!(disk.free > 0, "{disk:?}");
    assert_eq!(disk.labels, vec!["Filmek".to_owned()]);
    assert_eq!(disk.torrents, 2);
    assert_eq!(disk.zones.len(), 2);
    assert!(!disk.low, "the threshold is zero");
    assert!(report.unknown.is_empty(), "{report:?}");

    let _ = std::fs::remove_dir_all(&base);
}
