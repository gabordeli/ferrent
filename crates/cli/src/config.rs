//! CLI configuration: flags > environment > config file > defaults.
//!
//! The file holds one connection for everyday use plus named profiles
//! (`[[servers]]`) so a laptop can switch between daemons without env vars;
//! the GUI keeps the same shape in `gui.toml`.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// Used when neither a flag, the environment nor the file names a daemon.
pub const DEFAULT_SERVER: &str = "https://127.0.0.1:50051";

#[derive(Debug, Default, Deserialize, Serialize)]
struct ConfigFile {
    server: Option<String>,
    ca: Option<PathBuf>,
    token: Option<String>,
    /// Profile `ferrent-cli servers use` selected.
    active: Option<String>,
    #[serde(default)]
    servers: Vec<ServerEntry>,
}

/// One named daemon; the CLI counterpart of the GUI's server profiles.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ServerEntry {
    pub name: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

pub struct ResolvedConfig {
    pub server: String,
    pub ca: PathBuf,
    pub token: String,
}

pub fn resolve(
    cli_server: Option<String>,
    cli_ca: Option<PathBuf>,
    cli_token: Option<String>,
    cli_config: Option<PathBuf>,
) -> Result<ResolvedConfig, CliError> {
    let config_path = cli_config.unwrap_or_else(default_config_path);
    let file = load(&config_path)?;

    // A bare word names a profile; anything with a scheme is a URL.
    let requested = cli_server
        .or_else(|| env::var("FERRENT_SERVER").ok())
        .or_else(|| file.active.clone())
        .or_else(|| file.server.clone())
        .unwrap_or_else(|| DEFAULT_SERVER.to_owned());
    let profile = if requested.contains("://") {
        None
    } else {
        Some(
            file.servers
                .iter()
                .find(|entry| entry.name == requested)
                .ok_or_else(|| {
                    CliError::Config(format!(
                        "unknown server {requested:?}; run `ferrent-cli servers` to list the saved ones"
                    ))
                })?,
        )
    };
    let server = profile.map_or_else(|| requested.clone(), |entry| entry.server.clone());

    let ca = cli_ca
        .or_else(|| env::var_os("FERRENT_CA").map(PathBuf::from))
        .or_else(|| profile.and_then(|entry| entry.ca.clone()))
        .or(file.ca)
        .ok_or_else(|| {
            CliError::Config(
                "CA certificate required: use --ca, FERRENT_CA, or the config file".to_owned(),
            )
        })?;

    let token = cli_token
        .or_else(|| env::var("FERRENT_TOKEN").ok())
        .or_else(|| profile.and_then(|entry| entry.token.clone()))
        .or(file.token)
        .ok_or_else(|| {
            CliError::Config(
                "API token required: use --token, FERRENT_TOKEN, or the config file".to_owned(),
            )
        })?;

    Ok(ResolvedConfig { server, ca, token })
}

/// The saved profiles and the active name, for `ferrent-cli servers`.
pub fn profiles(
    cli_config: Option<PathBuf>,
) -> Result<(Vec<ServerEntry>, Option<String>), CliError> {
    let file = load(&cli_config.unwrap_or_else(default_config_path))?;
    Ok((file.servers, file.active))
}

/// Adds or replaces a profile; `use_it` also makes it the active one.
pub fn add_profile(
    cli_config: Option<PathBuf>,
    entry: ServerEntry,
    use_it: bool,
) -> Result<PathBuf, CliError> {
    let path = cli_config.unwrap_or_else(default_config_path);
    let mut file = load(&path)?;
    if let Some(existing) = file
        .servers
        .iter_mut()
        .find(|existing| existing.name == entry.name)
    {
        *existing = entry.clone();
    } else {
        file.servers.push(entry.clone());
    }
    if use_it {
        file.active = Some(entry.name.clone());
    }
    save(&path, &file)?;
    Ok(path)
}

/// Removes a profile, clearing `active` when it pointed at that name.
pub fn remove_profile(
    cli_config: Option<PathBuf>,
    name: &str,
) -> Result<(PathBuf, bool), CliError> {
    let path = cli_config.unwrap_or_else(default_config_path);
    let mut file = load(&path)?;
    let before = file.servers.len();
    file.servers.retain(|entry| entry.name != name);
    let removed = file.servers.len() != before;
    if file.active.as_deref() == Some(name) {
        file.active = None;
    }
    save(&path, &file)?;
    Ok((path, removed))
}

/// Makes an existing profile the default for every later command.
pub fn use_profile(cli_config: Option<PathBuf>, name: &str) -> Result<PathBuf, CliError> {
    let path = cli_config.unwrap_or_else(default_config_path);
    let mut file = load(&path)?;
    if !file.servers.iter().any(|entry| entry.name == name) {
        return Err(CliError::Config(format!(
            "unknown server {name:?}; run `ferrent-cli servers` to list the saved ones"
        )));
    }
    file.active = Some(name.to_owned());
    save(&path, &file)?;
    Ok(path)
}

fn load(path: &Path) -> Result<ConfigFile, CliError> {
    if !path.exists() {
        return Ok(ConfigFile::default());
    }
    let text = fs::read_to_string(path)?;
    toml::from_str(&text)
        .map_err(|error| CliError::Config(format!("invalid config {}: {error}", path.display())))
}

/// Writes the file with mode 0600: it holds API tokens.
fn save(path: &Path, file: &ConfigFile) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(file)
        .map_err(|error| CliError::Config(format!("cannot encode config: {error}")))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(path)?;
    handle.write_all(text.as_bytes())?;
    Ok(())
}

fn default_config_path() -> PathBuf {
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("ferrent").join("cli.toml");
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("ferrent")
            .join("cli.toml");
    }
    PathBuf::from("ferrent-cli.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ferrent-cli-config-{tag}-{}.toml",
            std::process::id()
        ))
    }

    #[test]
    fn a_bare_word_resolves_to_a_saved_profile() {
        let path = temp_file("profile");
        let _ = fs::remove_file(&path);
        add_profile(
            Some(path.clone()),
            ServerEntry {
                name: "media".to_owned(),
                server: "https://192.168.2.27:50051".to_owned(),
                ca: Some(PathBuf::from("/tmp/ca.pem")),
                token: Some("s3cret".to_owned()),
            },
            true,
        )
        .expect("add profile");

        let resolved = resolve(Some("media".to_owned()), None, None, Some(path.clone()))
            .expect("resolve by name");
        assert_eq!(resolved.server, "https://192.168.2.27:50051");
        assert_eq!(resolved.token, "s3cret");

        // The active profile answers without --server too.
        let active = resolve(None, None, None, Some(path.clone())).expect("active profile");
        assert_eq!(active.server, "https://192.168.2.27:50051");

        // A flag still wins over the profile's own values.
        let overridden = resolve(
            Some("media".to_owned()),
            None,
            Some("other".to_owned()),
            Some(path.clone()),
        )
        .expect("token override");
        assert_eq!(overridden.token, "other");

        let unknown = resolve(Some("nope".to_owned()), None, None, Some(path.clone()));
        assert!(matches!(unknown, Err(CliError::Config(_))), "unknown name");

        let (path, removed) = remove_profile(Some(path), "media").expect("remove");
        assert!(removed);
        let (saved, active) = profiles(Some(path.clone())).expect("profiles");
        assert!(saved.is_empty());
        assert_eq!(active, None);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn saving_keeps_the_flat_keys() {
        let path = temp_file("flat");
        let _ = fs::remove_file(&path);
        fs::write(
            &path,
            "server = \"https://example:50051\"\nca = \"/tmp/ca.pem\"\ntoken = \"t\"\n",
        )
        .expect("seed config");
        add_profile(
            Some(path.clone()),
            ServerEntry {
                name: "lab".to_owned(),
                server: "https://lab:50051".to_owned(),
                ca: None,
                token: Some("lab-token".to_owned()),
            },
            false,
        )
        .expect("add");
        let text = fs::read_to_string(&path).expect("read back");
        assert!(
            text.contains("server = \"https://example:50051\""),
            "{text}"
        );
        assert!(text.contains("[[servers]]"), "{text}");
        assert!(text.contains("name = \"lab\""), "{text}");
        let _ = fs::remove_file(path);
    }
}
