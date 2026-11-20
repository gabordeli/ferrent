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
    pub daemon: DaemonConfig,
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
        if self.login_per_minute == 0 {
            return Err("web.login_per_minute must be positive".to_owned());
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
}
