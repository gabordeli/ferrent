//! Configuration of the web gateway.
//!
//! The gateway sits in front of the daemon: nginx terminates TLS and proxies
//! to this process on loopback, so the daemon port never leaves the host.

use std::path::PathBuf;

use serde::Deserialize;

/// Where the gateway listens and how it reaches the daemon.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    /// Listen address of the plain-HTTP listener (loopback, behind nginx).
    pub listen: String,
    /// Login password; empty refuses to start.
    pub password: String,
    /// Reject every mutating request.
    pub read_only: bool,
    /// Session lifetime in seconds.
    pub session_ttl_secs: u64,
    /// SSE keep-alive interval in seconds.
    pub heartbeat_secs: u64,
    /// Window used to coalesce daemon deltas into one SSE message.
    pub coalesce_ms: u64,
    /// Login attempts allowed per minute per client.
    pub login_per_minute: u32,
    /// Optional directory with the built SPA; a placeholder is served without it.
    pub spa_dir: Option<PathBuf>,
    /// HTTPS for the gateway itself; without it the listener is plain HTTP.
    pub tls: Option<TlsConfig>,
    /// Web Push (VAPID) settings; disabled by default.
    pub push: PushConfig,
    pub daemon: DaemonConfig,
}

/// Web Push settings: the VAPID identity and where the subscriptions live.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PushConfig {
    /// Whether the gateway offers push at all.
    pub enabled: bool,
    /// VAPID contact claim; `mailto:` or `https:`.
    pub subject: String,
    /// PKCS8 PEM private key; generated on first start when missing.
    pub private_key: PathBuf,
    /// JSON store of browser subscriptions.
    pub subscriptions: PathBuf,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            subject: "mailto:admin@localhost".to_owned(),
            private_key: PathBuf::from("/var/lib/ferrent/vapid-private.pem"),
            subscriptions: PathBuf::from("/var/lib/ferrent/push-subscriptions.json"),
        }
    }
}

/// The gateway's own certificate.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    /// PEM certificate; generated when the file (or the key) is missing.
    pub cert: PathBuf,
    /// PEM private key, kept 0600.
    pub key: PathBuf,
    /// Extra subject alternative names (host name, LAN address).
    #[serde(default)]
    pub names: Vec<String>,
}

/// Daemon connection: TLS trust anchor and API token.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    /// gRPC endpoint of the daemon (`https://host:port`).
    pub url: String,
    /// PEM file with the daemon's self-signed certificate.
    pub ca: PathBuf,
    /// Bearer token of the daemon.
    pub token: String,
    /// Optional SHA-256 fingerprint of the CA; a mismatch refuses to start.
    pub fingerprint: Option<String>,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_owned(),
            password: String::new(),
            read_only: true,
            session_ttl_secs: 86_400,
            heartbeat_secs: 15,
            coalesce_ms: 1_000,
            login_per_minute: 5,
            spa_dir: None,
            tls: None,
            push: PushConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            url: "https://127.0.0.1:50129".to_owned(),
            ca: PathBuf::from("/var/lib/ferrent/tls-cert.pem"),
            token: String::new(),
            fingerprint: None,
        }
    }
}

impl WebConfig {
    /// Reads and validates a configuration file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Rejects a configuration the gateway cannot run with.
    pub fn validate(&self) -> Result<(), String> {
        if self.password.trim().is_empty() {
            return Err("web.password must not be empty".to_owned());
        }
        if self.listen.parse::<std::net::SocketAddr>().is_err() {
            return Err(format!("web.listen {:?} is not an address", self.listen));
        }
        if !self.daemon.url.starts_with("https://") {
            return Err("daemon.url must use https://".to_owned());
        }
        if !self.daemon.ca.is_file() {
            return Err(format!(
                "daemon.ca {} does not exist",
                self.daemon.ca.display()
            ));
        }
        if self.daemon.token.is_empty() {
            return Err("daemon.token must not be empty".to_owned());
        }
        if self.session_ttl_secs == 0 || self.heartbeat_secs == 0 || self.coalesce_ms == 0 {
            return Err(
                "session_ttl_secs, heartbeat_secs and coalesce_ms must be positive".to_owned(),
            );
        }
        if let Some(tls) = &self.tls {
            if tls.cert == tls.key {
                return Err("web.tls cert and key must be different files".to_owned());
            }
            if !crate::tls::is_pem(&tls.cert) || !crate::tls::is_pem(&tls.key) {
                return Err("web.tls cert and key must be .pem files".to_owned());
            }
        }
        if self.login_per_minute == 0 {
            return Err("web.login_per_minute must be positive".to_owned());
        }
        if self.push.enabled {
            if !(self.push.subject.starts_with("mailto:")
                || self.push.subject.starts_with("https://"))
            {
                return Err("push.subject must start with mailto: or https://".to_owned());
            }
            if self
                .push
                .private_key
                .extension()
                .and_then(|ext| ext.to_str())
                != Some("pem")
            {
                return Err("push.private_key must be a .pem file".to_owned());
            }
            if self.push.private_key == self.push.subscriptions {
                return Err(
                    "push.private_key and push.subscriptions must be different files".to_owned(),
                );
            }
        }
        Ok(())
    }

    /// Session lifetime.
    #[must_use]
    pub fn session_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.session_ttl_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> WebConfig {
        WebConfig {
            password: "secret".to_owned(),
            daemon: DaemonConfig {
                ca: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
                token: "token".to_owned(),
                ..DaemonConfig::default()
            },
            ..WebConfig::default()
        }
    }

    #[test]
    fn a_valid_configuration_passes() {
        valid().validate().expect("valid");
    }

    #[test]
    fn empty_passwords_and_plain_http_are_rejected() {
        let mut config = valid();
        config.password.clear();
        assert!(config.validate().is_err(), "empty password");

        let mut config = valid();
        config.daemon.url = "http://localhost:50129".to_owned();
        assert!(config.validate().is_err(), "plain http");

        let mut config = valid();
        config.daemon.ca = PathBuf::from("/definitely/not/here.pem");
        assert!(config.validate().is_err(), "missing CA");

        let mut config = valid();
        config.coalesce_ms = 0;
        assert!(config.validate().is_err(), "zero coalesce");
    }

    #[test]
    fn push_needs_a_contact_subject_and_separate_files() {
        let mut config = valid();
        config.push.enabled = true;
        assert!(config.validate().is_ok(), "enabled with defaults");

        let mut config = valid();
        config.push.enabled = true;
        config.push.subject = "admin@localhost".to_owned();
        assert!(config.validate().is_err(), "subject without scheme");

        let mut config = valid();
        config.push.enabled = true;
        config.push.private_key = config.push.subscriptions.clone();
        assert!(
            config.validate().is_err(),
            "the key would overwrite the store"
        );
    }
}
