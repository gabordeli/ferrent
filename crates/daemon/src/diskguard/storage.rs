//! Mount table, free space and filesystem identity.
//!
//! Everything is a pure function over the kernel mount table text (plus an
//! injected `statvfs`/`stat`), so bind mounts and pseudo filesystems are
//! testable without root.

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// Filesystems that are not worth showing in a storage panel.
pub const PSEUDO_FS: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nfsd",
    "nsfs",
    "overlay",
    "pstore",
    "proc",
    "ramfs",
    "rootfs",
    "rpc_pipefs",
    "securityfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// One entry of the kernel mount table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub mountpoint: PathBuf,
    pub fstype: String,
}

/// Total/free/used bytes of one filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiskStat {
    pub total: u64,
    pub free: u64,
    pub used: u64,
}

/// Parses `/proc/self/mounts` text.
#[must_use]
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    let mut mounts = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        mounts.push(Mount {
            device: unescape(fields[0]),
            mountpoint: normalise(Path::new(&unescape(fields[1]))),
            fstype: fields[2].to_owned(),
        });
    }
    mounts
}

/// Whether a filesystem type is a pseudo filesystem.
#[must_use]
pub fn is_pseudo(fstype: &str) -> bool {
    PSEUDO_FS.contains(&fstype)
}

/// Mount points worth reporting, in table order.
#[must_use]
pub fn real_mounts(mounts: &[Mount]) -> Vec<&Mount> {
    mounts
        .iter()
        .filter(|mount| !is_pseudo(&mount.fstype))
        .collect()
}

/// Reads the live mount table; an unreadable table yields an empty list.
#[must_use]
pub fn mount_table() -> Vec<Mount> {
    #[cfg(unix)]
    {
        std::fs::read_to_string("/proc/self/mounts")
            .map(|text| parse_mounts(&text))
            .unwrap_or_default()
    }
    #[cfg(windows)]
    {
        windows_drives()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

/// Drives with a mounted volume as a mount table, one entry per drive letter.
///
/// `/proc/self/mounts` has no Windows equivalent; each drive is its own
/// filesystem, so the drive root is the mount point and the volume serial is
/// the filesystem identity. A drive whose volume information cannot be read
/// (an empty card reader, a disconnected network share) is left out.
#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_drives() -> Vec<Mount> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

    let mask = unsafe { GetLogicalDrives() };
    let mut mounts = Vec::new();
    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let letter = char::from(b'A' + u8::try_from(index).unwrap_or(0));
        let root = format!("{letter}:\\");
        let Some(fstype) = volume_filesystem(&root) else {
            continue;
        };
        mounts.push(Mount {
            device: format!("{letter}:"),
            mountpoint: PathBuf::from(&root),
            fstype,
        });
    }
    mounts
}

#[allow(unsafe_code)]
#[cfg(windows)]
fn wide_str(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Filesystem name of a drive root, as Windows reports it (e.g. `NTFS`).
#[allow(unsafe_code)]
#[cfg(windows)]
fn volume_filesystem(root: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root = wide_str(root);
    let mut name = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            u32::try_from(name.len()).unwrap_or(0),
        )
    };
    if ok == 0 {
        return None;
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..end]).to_lowercase())
}

/// Filesystem id of a path (`st_dev`).
#[must_use]
pub fn device_id(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        std::fs::metadata(path).ok().map(|metadata| metadata.dev())
    }
    #[cfg(windows)]
    {
        // Match the Unix semantics: the identity comes from the path that
        // exists; `device_id_nearest` walks up for the not-yet-created case.
        if !path.exists() {
            return None;
        }
        volume_serial(&drive_root(path)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Drive root of a path (`C:\`), absolute paths only.
#[allow(unsafe_code)]
#[cfg(windows)]
fn drive_root(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    match absolute.components().next()? {
        Component::Prefix(prefix) => {
            let mut root = PathBuf::from(prefix.as_os_str());
            root.push("\\");
            Some(root)
        }
        Component::RootDir => Some(PathBuf::from("\\")),
        _ => None,
    }
}

/// Volume serial number of a drive root, the Windows `st_dev` equivalent.
#[allow(unsafe_code)]
#[cfg(windows)]
fn volume_serial(root: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root = wide_str(&root.to_string_lossy());
    let mut serial: u32 = 0;
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            &raw mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    (ok != 0).then_some(u64::from(serial))
}

/// Filesystem id of a path, walking up to an existing ancestor.
///
/// Torrent save paths may not exist yet; their directory usually does, so the
/// nearest ancestor is the right filesystem for counting.
#[must_use]
pub fn device_id_nearest(path: &Path) -> Option<u64> {
    let mut candidate = path;
    loop {
        if let Some(device) = device_id(candidate) {
            return Some(device);
        }
        candidate = candidate.parent()?;
    }
}

/// Free-space numbers of the filesystem holding a path.
#[must_use]
#[allow(unsafe_code)] // the Win32 disk-space call is FFI
pub fn disk_stat(path: &Path) -> Option<DiskStat> {
    #[cfg(unix)]
    {
        let stat = rustix::fs::statvfs(path).ok()?;
        let block = stat.f_frsize;
        Some(DiskStat {
            total: stat.f_blocks.saturating_mul(block),
            free: stat.f_bavail.saturating_mul(block),
            used: stat
                .f_blocks
                .saturating_sub(stat.f_bfree)
                .saturating_mul(block),
        })
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

        let wide = wide_str(&path.to_string_lossy());
        let mut free = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &raw mut free,
                &raw mut total,
                &raw mut total_free,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(DiskStat {
            // `free` is what *this* user may still write; `total_free` also
            // counts the reserved blocks, so `used` keeps them in.
            total,
            free,
            used: total.saturating_sub(total_free),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Free-space numbers of a path, walking up to an existing ancestor.
#[must_use]
pub fn disk_stat_nearest(path: &Path) -> Option<DiskStat> {
    let mut candidate = path;
    loop {
        if let Some(stat) = disk_stat(candidate) {
            return Some(stat);
        }
        candidate = candidate.parent()?;
    }
}

/// The longest mount point containing `path`.
///
/// A path that does not exist is normalised lexically, so an unmounted zone
/// still maps to its mount entry instead of failing.
#[must_use]
pub fn canonical_mount<'a>(path: &Path, mounts: &'a [Mount]) -> Option<&'a Mount> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| absolute(path));
    mounts
        .iter()
        .filter(|mount| covers(&resolved, &mount.mountpoint))
        .max_by_key(|mount| mount.mountpoint.as_os_str().len())
}

/// The longest mount point backed by `device`.
///
/// Bind mounts share a device, so the longest alias is the useful display
/// name for the filesystem.
#[must_use]
pub fn mount_for_device_with(
    device: u64,
    mounts: &[Mount],
    device_of: impl Fn(&Path) -> Option<u64>,
) -> Option<&Mount> {
    // A zone may live on a tmpfs (for example `/dev/shm`); only the extra
    // "offer every filesystem" list filters pseudo filesystems.
    mounts
        .iter()
        .filter(|mount| device_of(&mount.mountpoint) == Some(device))
        .max_by_key(|mount| mount.mountpoint.as_os_str().len())
}

/// [`mount_for_device_with`] against the real filesystem.
#[must_use]
pub fn mount_for_device(device: u64, mounts: &[Mount]) -> Option<Mount> {
    mount_for_device_with(device, mounts, device_id).cloned()
}

fn covers(path: &Path, mountpoint: &Path) -> bool {
    mountpoint == Path::new("/") || path == mountpoint || path.starts_with(mountpoint)
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        lexical(path)
    } else {
        std::env::current_dir()
            .map(|directory| lexical(&directory.join(path)))
            .unwrap_or_else(|_| lexical(path))
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn normalise(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.len() > 1 && text.ends_with('/') {
        PathBuf::from(text.trim_end_matches('/'))
    } else if text.is_empty() {
        PathBuf::from("/")
    } else {
        path.to_owned()
    }
}

fn unescape(field: &str) -> String {
    field
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\134", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
proc /proc proc rw,nosuid 0 0
sysfs /sys sysfs rw 0 0
/dev/sda1 / ext4 rw,relatime 0 0
tmpfs /run tmpfs rw 0 0
/dev/sdb1 /mnt/usbhdd2 ext4 rw 0 0
/dev/sdb1 /mnt/usbhdd2-alias ext4 rw 0 0
/dev/sdc1 /mnt/usbhdd3 ext4 rw 0 0
/dev/sdd1 /mnt/with\\040space ext4 rw 0 0
";

    #[test]
    fn the_mount_table_parses_and_unescapes() {
        let mounts = parse_mounts(TABLE);
        assert_eq!(mounts.len(), 8);
        assert_eq!(mounts[7].mountpoint, PathBuf::from("/mnt/with space"));
        assert_eq!(mounts[2].device, "/dev/sda1");
        assert_eq!(mounts[2].fstype, "ext4");
    }

    #[test]
    fn pseudo_filesystems_are_filtered() {
        let mounts = parse_mounts(TABLE);
        let real = real_mounts(&mounts);
        let types: Vec<&str> = real.iter().map(|mount| mount.fstype.as_str()).collect();
        assert_eq!(types, vec!["ext4", "ext4", "ext4", "ext4", "ext4"]);
        assert!(is_pseudo("tmpfs"));
        assert!(!is_pseudo("ext4"));
    }

    #[test]
    fn the_longest_mount_point_wins() {
        let mounts = parse_mounts(TABLE);
        let mount = canonical_mount(Path::new("/mnt/usbhdd2/Filmek"), &mounts).expect("mount");
        assert_eq!(mount.mountpoint, PathBuf::from("/mnt/usbhdd2"));

        // A path that does not exist still maps lexically.
        let mount =
            canonical_mount(Path::new("/mnt/usbhdd3/missing/_progress"), &mounts).expect("mount");
        assert_eq!(mount.mountpoint, PathBuf::from("/mnt/usbhdd3"));

        let mount = canonical_mount(Path::new("/etc"), &mounts).expect("mount");
        assert_eq!(mount.mountpoint, PathBuf::from("/"));
    }

    #[test]
    fn a_zone_on_a_tmpfs_still_finds_its_mount() {
        let mounts = parse_mounts(TABLE);
        let device_of = |path: &Path| match path.to_string_lossy().as_ref() {
            "/run" => Some(42),
            _ => None,
        };
        let mount = mount_for_device_with(42, &mounts, device_of).expect("tmpfs mount");
        assert_eq!(mount.mountpoint, PathBuf::from("/run"));
    }

    #[test]
    fn bind_mount_aliases_resolve_to_the_longest_mount_point() {
        let mounts = parse_mounts(TABLE);
        let dev = |path: &Path| match path.to_string_lossy().as_ref() {
            "/mnt/usbhdd2" | "/mnt/usbhdd2-alias" => Some(7),
            "/" => Some(1),
            _ => None,
        };
        let mount = mount_for_device_with(7, &mounts, dev).expect("device");
        assert_eq!(mount.mountpoint, PathBuf::from("/mnt/usbhdd2-alias"));
    }

    #[test]
    fn real_free_space_and_device_ids_are_read() {
        let stat = disk_stat(Path::new("/tmp")).expect("statvfs");
        assert!(stat.total > 0, "{stat:?}");
        assert!(stat.free <= stat.total, "{stat:?}");
        stat_vs_statvfs(&stat);
        assert!(device_id(Path::new("/tmp")).is_some());
        assert_eq!(
            device_id_nearest(Path::new("/tmp/ferrent/not/created/yet")),
            device_id(Path::new("/tmp"))
        );
        assert!(disk_stat(Path::new("/definitely/not/here")).is_none());
        assert!(device_id(Path::new("/definitely/not/here")).is_none());
    }

    #[test]
    fn nearest_disk_stat_walks_up_to_an_existing_ancestor() {
        let missing = std::env::temp_dir()
            .join("ferrent-not-created-yet")
            .join("sub");
        let stat = disk_stat_nearest(&missing).expect("ancestor stat");
        assert!(stat.total > 0, "{stat:?}");
        stat_vs_statvfs(&stat);
    }

    fn stat_vs_statvfs(stat: &DiskStat) {
        // `used` uses all blocks, so it can exceed `total - free` (reserved
        // blocks are not available to unprivileged users).
        assert!(stat.used <= stat.total, "{stat:?}");
    }
}
