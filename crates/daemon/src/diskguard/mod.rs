//! DiskGuard: storage zones, free space and the label → staging resolution.
//!
//! The daemon writes into a *staging* folder per filesystem and moves finished
//! torrents into the label's *library* folder. Both folders live in the same
//! zone list, so the completion move stays a rename on one filesystem.

pub mod storage;
pub mod zones;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use storage::{
    DiskStat, Mount, canonical_mount, device_id, device_id_nearest, disk_stat, mount_for_device,
    mount_for_device_with, mount_table, parse_mounts, real_mounts,
};
pub use zones::{
    Zone, ZoneError, library_zone, resolve_staging, resolve_staging_with, validate_zones,
};

/// Upper bound for one storage scan; a stuck NFS mount must not block.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(5);

/// One zone in the storage payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneReport {
    pub path: PathBuf,
    pub label: Option<String>,
    pub staging: bool,
    pub enabled: bool,
}

impl ZoneReport {
    fn from_zone(zone: &Zone) -> Self {
        Self {
            path: zone.path.clone(),
            label: zone.label.clone(),
            staging: zone.staging,
            enabled: zone.enabled,
        }
    }
}

/// One filesystem with the zones that live on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReport {
    /// Longest mount point backed by the device.
    pub mount: PathBuf,
    pub device: String,
    pub fstype: String,
    pub total: u64,
    pub free: u64,
    pub used: u64,
    /// `free < min_free_bytes`; never set when the threshold is zero.
    pub low: bool,
    pub zones: Vec<ZoneReport>,
    /// Labels whose library zone lives on this filesystem.
    pub labels: Vec<String>,
    /// Torrents whose save path is on this filesystem.
    pub torrents: usize,
}

/// The storage payload: one entry per filesystem plus the paths that could
/// not be mapped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageReport {
    pub disks: Vec<DiskReport>,
    pub unknown: Vec<PathBuf>,
}

impl StorageReport {
    /// Free bytes across every reported filesystem.
    #[must_use]
    pub fn total_free(&self) -> u64 {
        self.disks.iter().map(|disk| disk.free).sum()
    }

    /// Number of filesystems below their threshold.
    #[must_use]
    pub fn low_count(&self) -> usize {
        self.disks.iter().filter(|disk| disk.low).count()
    }
}

/// Builds the storage payload from the zone list and a mount table.
///
/// `torrents` pairs a torrent id with its save path; only the filesystem
/// counts are used. `stat` and `device` are injected so tests can fake
/// filesystems.
pub fn report_with(
    zones: &[Zone],
    torrents: &[(u64, PathBuf)],
    mounts: &[Mount],
    min_free: u64,
    stat: impl Fn(&Path) -> Option<DiskStat>,
    device: impl Fn(&Path) -> Option<u64>,
    torrent_device: impl Fn(&Path) -> Option<u64>,
) -> StorageReport {
    let mut groups: std::collections::BTreeMap<u64, Vec<ZoneReport>> =
        std::collections::BTreeMap::new();
    let mut unknown = Vec::new();
    for zone in zones {
        match device(&zone.path) {
            Some(filesystem) => groups
                .entry(filesystem)
                .or_default()
                .push(ZoneReport::from_zone(zone)),
            None => unknown.push(zone.path.clone()),
        }
    }

    let mut torrent_counts: std::collections::BTreeMap<u64, usize> =
        std::collections::BTreeMap::new();
    for (_, path) in torrents {
        if let Some(filesystem) = torrent_device(path) {
            *torrent_counts.entry(filesystem).or_default() += 1;
        }
    }

    let mut disks = Vec::with_capacity(groups.len());
    for (filesystem, mut zones) in groups {
        let Some(mount) = mount_for_device_with(filesystem, mounts, &device) else {
            unknown.extend(zones.into_iter().map(|zone| zone.path));
            continue;
        };
        let Some(stat) = stat(&mount.mountpoint) else {
            unknown.extend(zones.into_iter().map(|zone| zone.path));
            continue;
        };
        zones.sort_by(|left, right| left.path.cmp(&right.path));
        let mut labels: Vec<String> = zones
            .iter()
            .filter(|zone| !zone.staging)
            .filter_map(|zone| zone.label.clone())
            .collect();
        labels.sort();
        labels.dedup();
        disks.push(DiskReport {
            mount: mount.mountpoint.clone(),
            device: mount.device.clone(),
            fstype: mount.fstype.clone(),
            total: stat.total,
            free: stat.free,
            used: stat.used,
            low: min_free > 0 && stat.free < min_free,
            torrents: torrent_counts.get(&filesystem).copied().unwrap_or(0),
            labels,
            zones,
        });
    }
    disks.sort_by(|left, right| left.mount.cmp(&right.mount));
    unknown.sort();
    unknown.dedup();
    StorageReport { disks, unknown }
}

/// Reads the live mount table and free space.
///
/// Returns `None` when the scan did not finish within [`SCAN_TIMEOUT`]; the
/// caller keeps its last payload. A timed-out blocking scan keeps running in
/// the blocking pool, which is harmless for a periodic poll.
pub async fn snapshot(
    zones: &[Zone],
    torrents: &[(u64, PathBuf)],
    min_free: u64,
) -> Option<StorageReport> {
    let zones = zones.to_vec();
    let torrents = torrents.to_vec();
    let task = tokio::task::spawn_blocking(move || {
        let mounts = mount_table();
        report_with(
            &zones,
            &torrents,
            &mounts,
            min_free,
            disk_stat,
            device_id,
            device_id_nearest,
        )
    });
    tokio::time::timeout(SCAN_TIMEOUT, task).await.ok()?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
/dev/sda1 / ext4 rw 0 0
tmpfs /run tmpfs rw 0 0
/dev/sdb1 /mnt/usbhdd2 ext4 rw 0 0
/dev/sdb1 /mnt/usbhdd2-alias ext4 rw 0 0
/dev/sdc1 /mnt/usbhdd3 ext4 rw 0 0
";

    fn fake_device(path: &Path) -> Option<u64> {
        let text = path.to_string_lossy();
        if text.starts_with("/mnt/usbhdd2") {
            Some(2)
        } else if text.starts_with("/mnt/usbhdd3") {
            Some(3)
        } else if text.starts_with("/mnt/missing") {
            None
        } else {
            Some(1)
        }
    }

    fn fake_stat(path: &Path) -> Option<DiskStat> {
        match path.to_string_lossy().as_ref() {
            "/mnt/usbhdd2-alias" => Some(DiskStat {
                total: 1_000,
                free: 100,
                used: 900,
            }),
            "/mnt/usbhdd3" => Some(DiskStat {
                total: 2_000,
                free: 1_500,
                used: 500,
            }),
            _ => Some(DiskStat {
                total: 10,
                free: 10,
                used: 0,
            }),
        }
    }

    fn zones() -> Vec<Zone> {
        vec![
            Zone::library("/mnt/usbhdd2/Filmek", "Filmek"),
            Zone::library("/mnt/usbhdd2/Sorozatok", "Sorozatok"),
            Zone::staging("/mnt/usbhdd2/_progress"),
            Zone::library("/mnt/usbhdd3/for-seed", "seed"),
            Zone::staging("/mnt/usbhdd3/_progress"),
            Zone::library("/mnt/missing/thing", "ghost"),
        ]
    }

    #[test]
    fn one_disk_lists_all_of_its_zones() {
        let mounts = parse_mounts(TABLE);
        let report = report_with(
            &zones(),
            &[],
            &mounts,
            0,
            fake_stat,
            fake_device,
            fake_device,
        );
        assert_eq!(report.disks.len(), 2, "{report:?}");
        let first = &report.disks[0];
        assert_eq!(first.mount, PathBuf::from("/mnt/usbhdd2-alias"));
        assert_eq!(first.device, "/dev/sdb1");
        assert_eq!(first.zones.len(), 3);
        assert_eq!(first.zones[0].path, PathBuf::from("/mnt/usbhdd2/Filmek"));
        assert_eq!(first.zones[2].path, PathBuf::from("/mnt/usbhdd2/_progress"));
        assert!(first.zones[2].staging);
        assert_eq!(report.disks[1].mount, PathBuf::from("/mnt/usbhdd3"));
        assert_eq!(report.disks[1].zones.len(), 2);
        assert_eq!(
            first.labels,
            vec!["Filmek".to_owned(), "Sorozatok".to_owned()]
        );
        assert_eq!(report.unknown, vec![PathBuf::from("/mnt/missing/thing")]);
    }

    #[test]
    fn torrents_are_counted_per_filesystem() {
        let mounts = parse_mounts(TABLE);
        let torrents = vec![
            (1, PathBuf::from("/mnt/usbhdd2/_progress/one")),
            (2, PathBuf::from("/mnt/usbhdd2/Filmek/two")),
            (3, PathBuf::from("/mnt/usbhdd3/for-seed/three")),
            (4, PathBuf::from("/mnt/elsewhere/four")),
        ];
        let report = report_with(
            &zones(),
            &torrents,
            &mounts,
            0,
            fake_stat,
            fake_device,
            fake_device,
        );
        assert_eq!(report.disks[0].torrents, 2, "{report:?}");
        assert_eq!(report.disks[1].torrents, 1, "{report:?}");
    }

    #[test]
    fn low_follows_the_threshold_and_is_off_by_default() {
        let mounts = parse_mounts(TABLE);
        let report = report_with(
            &zones(),
            &[],
            &mounts,
            0,
            fake_stat,
            fake_device,
            fake_device,
        );
        assert_eq!(report.low_count(), 0, "no threshold, no low");

        let report = report_with(
            &zones(),
            &[],
            &mounts,
            500,
            fake_stat,
            fake_device,
            fake_device,
        );
        assert_eq!(report.low_count(), 1, "{report:?}");
        assert!(report.disks[0].low);
        assert!(!report.disks[1].low);
        assert_eq!(report.total_free(), 1_600);
    }

    #[test]
    fn a_failed_stat_makes_the_zones_unknown() {
        let mounts = parse_mounts(TABLE);
        let report = report_with(
            &zones(),
            &[],
            &mounts,
            0,
            |path| (path != Path::new("/mnt/usbhdd3")).then(DiskStat::default),
            fake_device,
            fake_device,
        );
        assert!(
            report
                .unknown
                .contains(&PathBuf::from("/mnt/usbhdd3/for-seed"))
        );
        assert!(
            report
                .unknown
                .contains(&PathBuf::from("/mnt/usbhdd3/_progress"))
        );
    }

    #[test]
    fn a_thousand_zones_are_grouped_quickly() {
        let mounts = parse_mounts(TABLE);
        let mut zones = Vec::with_capacity(1_000);
        for index in 0..1_000 {
            zones.push(Zone::library(
                format!("/mnt/usbhdd2/dir{index}"),
                format!("l{index}"),
            ));
        }
        let started = std::time::Instant::now();
        let report = report_with(&zones, &[], &mounts, 0, fake_stat, fake_device, fake_device);
        let elapsed = started.elapsed();
        assert_eq!(report.disks.len(), 1);
        assert_eq!(report.disks[0].zones.len(), 1_000);
        assert!(
            elapsed < Duration::from_millis(50),
            "grouping took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn the_live_snapshot_reads_the_real_filesystem() {
        // `/tmp` may be a tmpfs (filtered as pseudo); the build directory is
        // always on the root filesystem.
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("ferrent-zones-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("dir");
        let zones = vec![
            Zone::library(directory.join("library"), "test"),
            Zone::staging(&directory),
        ];
        let report = snapshot(&zones, &[], 0).await.expect("snapshot");
        assert!(!report.disks.is_empty(), "{report:?}");
        // The library folder does not exist yet, so it is reported as unknown.
        assert!(
            report.unknown.contains(&directory.join("library")),
            "{report:?}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
