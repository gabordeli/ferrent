//! HTTPS for the gateway itself.
//!
//! The daemon's certificate is issued for `localhost`, so a phone cannot use
//! it. The gateway therefore keeps its own pair: generated on first start with
//! the configured names (host name and LAN address) and reused afterwards.
//! The fingerprint is printed at startup, so the browser prompt can be checked.

use std::fs;
use std::net::IpAddr;
use std::path::Path;

use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertificateParams, KeyPair};

use crate::config::TlsConfig;

/// A ready TLS configuration plus its identity.
pub struct WebTls {
    pub config: RustlsConfig,
    pub fingerprint: String,
    pub cert_path: std::path::PathBuf,
}

/// Loads the certificate/key pair, generating one when it is missing.
pub async fn load_or_generate(settings: &TlsConfig) -> Result<WebTls, String> {
    // axum-server builds its own rustls config; pick the ring provider once.
    let _ = rustls::crypto::ring::default_provider().install_default();
    if !settings.cert.exists() || !settings.key.exists() {
        let names = names_for(settings);
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let params = CertificateParams::new(names).map_err(|error| error.to_string())?;
        let cert = params
            .self_signed(&key)
            .map_err(|error| error.to_string())?;
        if let Some(parent) = settings.cert.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        fs::write(&settings.cert, cert.pem())
            .map_err(|error| format!("cannot write {}: {error}", settings.cert.display()))?;
        let key_pem = key.serialize_pem();
        ferrent_platform::write_secret(&settings.key, key_pem.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", settings.key.display()))?;
    }

    let cert_pem = fs::read_to_string(&settings.cert)
        .map_err(|error| format!("cannot read {}: {error}", settings.cert.display()))?;
    let key_pem = fs::read_to_string(&settings.key)
        .map_err(|error| format!("cannot read {}: {error}", settings.key.display()))?;
    let fingerprint = ferrent_protocol::tls::fingerprint_from_pem(&cert_pem)?;
    let config = RustlsConfig::from_pem(cert_pem.into_bytes(), key_pem.into_bytes())
        .await
        .map_err(|error| error.to_string())?;
    Ok(WebTls {
        config,
        fingerprint,
        cert_path: settings.cert.clone(),
    })
}

/// Subject alternative names: the configured ones, `localhost` and the literal
/// addresses, deduplicated.
fn names_for(settings: &TlsConfig) -> Vec<String> {
    let mut names = settings.names.clone();
    names.push("localhost".to_owned());
    for address in ["127.0.0.1", "::1"] {
        names.push(address.to_owned());
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| {
            !name.is_empty()
                && (name.parse::<IpAddr>().is_ok()
                    || name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        })
        .collect()
}

/// Whether a path looks like a certificate file (for the config check).
#[must_use]
pub fn is_pem(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pem"))
}

/// SHA-256 of a certificate file, for documentation.
#[must_use]
pub fn fingerprint_of(path: &Path) -> Option<String> {
    let pem = fs::read_to_string(path).ok()?;
    ferrent_protocol::tls::fingerprint_from_pem(&pem).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn settings(dir: &Path) -> TlsConfig {
        TlsConfig {
            cert: dir.join("web-cert.pem"),
            key: dir.join("web-key.pem"),
            names: vec!["ferrent.local".to_owned(), "192.168.2.65".to_owned()],
        }
    }

    #[tokio::test]
    async fn a_missing_pair_is_generated_and_reused() {
        let dir = std::env::temp_dir().join(format!("ferrent-web-tls-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = settings(&dir);
        let first = load_or_generate(&settings).await.expect("generate");
        assert!(settings.cert.is_file());
        assert!(settings.key.is_file());
        let mode = fs::metadata(&settings.key)
            .expect("key")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the key is private");
        assert_eq!(first.fingerprint.len(), 95, "colon-separated SHA-256");

        let second = load_or_generate(&settings).await.expect("reload");
        assert_eq!(first.fingerprint, second.fingerprint, "reused");
        assert_eq!(
            fingerprint_of(&settings.cert).as_deref(),
            Some(first.fingerprint.as_str())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_filtered_and_deduplicated() {
        let settings = TlsConfig {
            cert: PathBuf::from("/tmp/c.pem"),
            key: PathBuf::from("/tmp/k.pem"),
            names: vec![
                "localhost".to_owned(),
                "192.168.2.65".to_owned(),
                "bad name".to_owned(),
                String::new(),
            ],
        };
        let names = names_for(&settings);
        assert!(names.contains(&"localhost".to_owned()));
        assert!(names.contains(&"192.168.2.65".to_owned()));
        assert!(!names.iter().any(|name| name.contains(' ')), "{names:?}");
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn sha256_needs_a_certificate() {
        assert!(fingerprint_of(Path::new("/definitely/not/here.pem")).is_none());
    }
}
