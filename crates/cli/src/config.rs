//! CLI configuration: flags > environment > config file > defaults.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::CliError;

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    server: Option<String>,
    ca: Option<PathBuf>,
    token: Option<String>,
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

    let server = cli_server
        .or_else(|| env::var("FERRENT_SERVER").ok())
        .or(file.server)
        .unwrap_or_else(|| "https://127.0.0.1:50051".to_owned());

    let ca = cli_ca
        .or_else(|| env::var_os("FERRENT_CA").map(PathBuf::from))
        .or(file.ca)
        .ok_or_else(|| {
            CliError::Config(
                "CA certificate required: use --ca, FERRENT_CA, or the config file".to_owned(),
            )
        })?;

    let token = cli_token
        .or_else(|| env::var("FERRENT_TOKEN").ok())
        .or(file.token)
        .ok_or_else(|| {
            CliError::Config(
                "API token required: use --token, FERRENT_TOKEN, or the config file".to_owned(),
            )
        })?;

    Ok(ResolvedConfig { server, ca, token })
}

fn load(path: &Path) -> Result<ConfigFile, CliError> {
    if !path.exists() {
        return Ok(ConfigFile::default());
    }
    let text = fs::read_to_string(path)?;
    toml::from_str(&text)
        .map_err(|error| CliError::Config(format!("invalid config {}: {error}", path.display())))
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
