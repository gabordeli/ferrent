//! Platform file locations and secret handling shared by the daemon, the CLI
//! and the GUI.
//!
//! Linux keeps the XDG layout for client configuration and the packaging
//! layout (`/etc/ferrent`, `/var/lib/ferrent`) for the daemon. Windows puts
//! the daemon under `%ProgramData%\ferrent` and client configuration under
//! `%APPDATA%\ferrent`, whose ACL already hides the tokens the files carry
//! from other users.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The user's home directory: `$HOME` on Unix, `%USERPROFILE%` (falling back
/// to `$HOME` for shells that set it) on Windows.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from)
}

/// Expands a leading `~`, `~/` or `~\` from [`home_dir`].
///
/// A path inside a config file never passes through a shell, so
/// `~/.config/...` would otherwise be a literal directory name (FERRENT-16).
/// Anything after the tilde that is not a separator is left alone, so `~user`
/// stays as written.
#[must_use]
pub fn expand_home(path: PathBuf) -> PathBuf {
    let expanded = path.to_str().and_then(expand_tilde);
    expanded.unwrap_or(path)
}

fn expand_tilde(text: &str) -> Option<PathBuf> {
    let rest = text.strip_prefix('~')?;
    if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
        return None;
    }
    let home = home_dir()?;
    Some(if rest.is_empty() {
        home
    } else {
        home.join(rest.trim_start_matches(['/', '\\']))
    })
}

/// The per-user Ferrent configuration directory: `$XDG_CONFIG_HOME/ferrent`
/// or `$HOME/.config/ferrent` on Unix, `%APPDATA%\ferrent` on Windows.
///
/// `None` when the environment gives no base; callers keep their working
/// directory fallback.
#[must_use]
pub fn user_config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("APPDATA").map(PathBuf::from),
        home_dir(),
    )
}

fn config_dir_from(
    xdg_config_home: Option<OsString>,
    appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    #[cfg(windows)]
    let base = {
        let _ = (xdg_config_home, home);
        appdata
    };
    #[cfg(not(windows))]
    let base = {
        let _ = appdata;
        xdg_config_home
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".config")))
    };
    base.map(|base| base.join("ferrent"))
}

/// `%ProgramData%\ferrent` on Windows; `None` on Unix, where the daemon uses
/// absolute `/etc/ferrent` paths.
#[must_use]
pub fn windows_program_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        Some(base.join("ferrent"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Writes a file that carries a token or a key: `0600` on Unix, including
/// when the file already existed with a looser mode.
///
/// On Windows the file inherits its parent's ACL. Keep per-user secrets under
/// `%APPDATA%` (the profile already protects them) and machine-wide ones under
/// `%ProgramData%` followed by [`harden_machine_secret`].
pub fn write_secret(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(path)?;
    handle.write_all(contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Restricts a machine-wide secret to SYSTEM and the administrators.
///
/// The `.deb` postinst creates the token `0640 root:ferrent`, and the daemon
/// may not own the file, so Unix does nothing here. On Windows the script
/// that writes the token inherits `%ProgramData%`'s ACL, which lets every
/// user read it; this tightens the file the daemon is about to use.
///
/// `icacls` takes well-known SIDs on purpose: on a translated Windows the
/// group is called `Rendszergazdák`, and the English name would not resolve.
/// A failure is a warning, not a startup error — a token the daemon can read
/// still works, it is only readable by others too.
pub fn harden_machine_secret(path: &Path) {
    #[cfg(windows)]
    {
        let outcome = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg("*S-1-5-18:(F)")
            .arg("*S-1-5-32-544:(F)")
            .status();
        match outcome {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("warning: icacls exited with {status} on {}", path.display()),
            Err(error) => eprintln!(
                "warning: cannot restrict {} to SYSTEM and Administrators: {error}",
                path.display()
            ),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_tilde_expands_from_home() {
        let home = home_dir().expect("HOME in the test environment");
        assert_eq!(
            expand_home(PathBuf::from("~/a/b")),
            home.join("a").join("b")
        );
        assert_eq!(expand_home(PathBuf::from("~")), home);
    }

    #[test]
    fn a_tilde_that_is_not_a_prefix_stays_alone() {
        assert_eq!(
            expand_home(PathBuf::from("~other/x")),
            PathBuf::from("~other/x")
        );
        assert_eq!(
            expand_home(PathBuf::from("/a/~/b")),
            PathBuf::from("/a/~/b")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_unix_config_dir_prefers_xdg_and_falls_back_to_home() {
        assert_eq!(
            config_dir_from(Some(OsString::from("/xdg")), None, None),
            Some(PathBuf::from("/xdg/ferrent"))
        );
        assert_eq!(
            config_dir_from(None, None, Some(PathBuf::from("/home/u"))),
            Some(PathBuf::from("/home/u/.config/ferrent"))
        );
        assert_eq!(config_dir_from(None, None, None), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_config_dir_lives_below_appdata() {
        assert_eq!(
            config_dir_from(
                None,
                Some(PathBuf::from(r"C:\Users\u\AppData\Roaming")),
                None
            ),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming\ferrent"))
        );
        assert_eq!(config_dir_from(None, None, None), None);
        let program_data = windows_program_data_dir().expect("ProgramData on Windows");
        assert!(program_data.ends_with("ferrent"));
    }

    #[test]
    fn writing_a_secret_leaves_content_and_a_private_mode() {
        let dir = std::env::temp_dir().join(format!(
            "ferrent-platform-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = dir.join("nested").join("secret.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(path.parent().expect("parent")).expect("seed dir");
            std::fs::write(&path, "old").expect("seed file");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                .expect("seed mode");
        }

        write_secret(&path, b"token = \"abc\"\n").expect("write");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "token = \"abc\"\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}
