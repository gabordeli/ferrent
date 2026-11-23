//! GUI configuration: server profiles and pinned certificate fingerprints.
//!
//! The file is `~/.config/ferrent/gui.toml` (or `$XDG_CONFIG_HOME`) and is
//! written with mode 0600 because it contains API tokens.

use std::fs;
use std::io;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuiConfig {
    #[serde(default)]
    pub profiles: Vec<ServerProfile>,
    #[serde(default)]
    pub active: Option<String>,
    /// `dark` (default) or `light`.
    #[serde(default)]
    pub theme: Option<String>,
    /// Accent colour as `#RRGGBB`; the primary/seeding colour follows it.
    #[serde(default)]
    pub accent: Option<String>,
    /// UI language: `hu` (default) or `en`.
    #[serde(default)]
    pub lang: Option<String>,
}

impl GuiConfig {
    /// The palette described by the config; unknown values fall back to dark.
    #[must_use]
    pub fn palette(&self) -> crate::theme::Palette {
        let base = match self.theme.as_deref() {
            Some("light") => crate::theme::Palette::light(),
            _ => crate::theme::Palette::dark(),
        };
        match self.accent.as_deref().and_then(parse_hex) {
            Some(accent) => base.with_accent(accent),
            None => base,
        }
    }

    /// Stores the chosen theme and accent.
    pub fn set_appearance(&mut self, theme: &str, accent: Option<&str>) {
        self.theme = Some(theme.to_owned());
        self.accent = accent.map(str::to_owned);
    }

    /// The UI language: `FERRENT_LANG` wins, then the config, then Hungarian.
    #[must_use]
    pub fn language(&self) -> crate::i18n::Lang {
        if let Ok(value) = std::env::var("FERRENT_LANG") {
            if !value.trim().is_empty() {
                return crate::i18n::Lang::parse(&value);
            }
        }
        match self.lang.as_deref() {
            Some(value) => crate::i18n::Lang::parse(value),
            None => crate::i18n::Lang::default(),
        }
    }
}

/// Parses `#RRGGBB` into a colour; alpha is not supported here.
#[must_use]
pub fn parse_hex(value: &str) -> Option<iced::Color> {
    let value = value.trim().strip_prefix('#')?;
    if value.len() != 6 {
        return None;
    }
    let channel = |offset: usize| u8::from_str_radix(&value[offset..offset + 2], 16).ok();
    Some(iced::Color::from_rgb8(
        channel(0)?,
        channel(2)?,
        channel(4)?,
    ))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerProfile {
    pub name: String,
    pub server: String,
    pub ca: PathBuf,
    pub token: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinDecision {
    First,
    Match,
    Mismatch { expected: String },
}

impl GuiConfig {
    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid GUI config {}: {error}", path.display()),
            )
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(text.as_bytes())?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        fs::write(path, text)?;
        Ok(())
    }

    #[must_use]
    pub fn default_path() -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(dir).join("ferrent").join("gui.toml");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".config")
                .join("ferrent")
                .join("gui.toml");
        }
        PathBuf::from("ferrent-gui.toml")
    }

    #[must_use]
    pub fn profile(&self, name: &str) -> Option<&ServerProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    pub fn upsert(&mut self, profile: ServerProfile) {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.name == profile.name)
        {
            *existing = profile.clone();
        } else {
            self.profiles.push(profile.clone());
        }
        self.active = Some(profile.name);
    }

    #[must_use]
    pub fn pin_decision(&self, profile: &ServerProfile, actual: &str) -> PinDecision {
        match profile.fingerprint.as_deref() {
            None => PinDecision::First,
            Some(expected) if expected == actual => PinDecision::Match,
            Some(expected) => PinDecision::Mismatch {
                expected: expected.to_owned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ferrent-gui-config-{name}-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    fn profile() -> ServerProfile {
        ServerProfile {
            name: "local".to_owned(),
            server: "https://localhost:50051".to_owned(),
            ca: PathBuf::from("/tmp/ca.pem"),
            token: "secret".to_owned(),
            fingerprint: None,
        }
    }

    #[test]
    fn profiles_roundtrip_and_file_is_0600() {
        let path = temp_path("roundtrip");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&path, "old").expect("seed config");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("seed mode");
        }
        let mut config = GuiConfig::default();
        config.upsert(profile());
        config.save(&path).expect("save");

        let loaded = GuiConfig::load(&path).expect("load");
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].token, "secret");
        assert_eq!(loaded.active.as_deref(), Some("local"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn pin_decisions() {
        let config = GuiConfig::default();
        let mut profile = profile();
        assert_eq!(config.pin_decision(&profile, "aa:bb"), PinDecision::First);

        profile.fingerprint = Some("aa:bb".to_owned());
        assert_eq!(config.pin_decision(&profile, "aa:bb"), PinDecision::Match);
        assert_eq!(
            config.pin_decision(&profile, "cc:dd"),
            PinDecision::Mismatch {
                expected: "aa:bb".to_owned()
            }
        );
    }
}
