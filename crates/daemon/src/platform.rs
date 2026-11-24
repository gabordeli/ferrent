//! Where the daemon keeps its files on each platform, and what protects the
//! token.
//!
//! Unix keeps the packaging layout (`/etc/ferrent`, `/var/lib/ferrent`). On
//! Windows the same three files live under `%ProgramData%\ferrent`, the
//! directory the service script creates, so a bare `ferrentd.exe` reads what
//! the service wrote instead of inventing a path next to the binary.

use std::path::PathBuf;

#[cfg(not(windows))]
const UNIX_CONFIG: &str = "/etc/ferrent/config.toml";
#[cfg(not(windows))]
const UNIX_TOKEN: &str = "/etc/ferrent/token";
#[cfg(not(windows))]
const UNIX_DATA: &str = "./ferrent-data";

#[cfg(windows)]
fn ferrent_dir() -> PathBuf {
    ferrent_platform::windows_program_data_dir().expect("Windows has %ProgramData%")
}

/// The default configuration file; `$FERRENT_CONFIG` overrides it.
#[cfg(windows)]
#[must_use]
pub fn default_config_path() -> PathBuf {
    ferrent_dir().join("config.toml")
}

/// The default configuration file; `$FERRENT_CONFIG` overrides it.
#[cfg(not(windows))]
#[must_use]
pub fn default_config_path() -> PathBuf {
    PathBuf::from(UNIX_CONFIG)
}

#[cfg(windows)]
#[must_use]
pub fn default_token_file() -> PathBuf {
    ferrent_dir().join("token")
}

#[cfg(not(windows))]
#[must_use]
pub fn default_token_file() -> PathBuf {
    PathBuf::from(UNIX_TOKEN)
}

#[cfg(windows)]
#[must_use]
pub fn default_data_dir() -> PathBuf {
    ferrent_dir().join("data")
}

#[cfg(not(windows))]
#[must_use]
pub fn default_data_dir() -> PathBuf {
    PathBuf::from(UNIX_DATA)
}

/// Whether the Deluge import is tested on this platform.
///
/// The parser is path-driven and the formats are platform-independent, but the
/// Windows Deluge layout, its plugin configs and the drive-letter `save_path`
/// values are unverified until the VM run; the Windows GUI hides the page while
/// `ferrent-cli import-deluge` stays available for the test.
#[cfg(windows)]
#[must_use]
pub fn import_supported() -> bool {
    false
}

#[cfg(not(windows))]
#[must_use]
pub fn import_supported() -> bool {
    true
}

/// Restricts the token file to the service account and the administrators.
///
/// The `.deb` postinst creates the file `0640 root:ferrent`. On Windows the
/// script that writes it inherits `%ProgramData%`'s ACL, which lets every user
/// read it, so the daemon tightens the file it is about to use.
pub use ferrent_platform::harden_machine_secret as harden_secret_file;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn unix_defaults_keep_the_packaging_layout() {
        assert_eq!(
            default_config_path(),
            PathBuf::from("/etc/ferrent/config.toml")
        );
        assert_eq!(default_token_file(), PathBuf::from("/etc/ferrent/token"));
        assert_eq!(default_data_dir(), PathBuf::from("./ferrent-data"));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_unix_import_is_supported() {
        assert!(import_supported());
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_import_waits_for_the_vm_run() {
        assert!(!import_supported());
    }

    #[cfg(windows)]
    #[test]
    fn windows_defaults_live_below_program_data() {
        let root = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("ferrent");
        assert_eq!(default_config_path(), root.join("config.toml"));
        assert_eq!(default_token_file(), root.join("token"));
        assert_eq!(default_data_dir(), root.join("data"));
    }
}
