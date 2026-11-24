//! Self-signed TLS material for the daemon.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use rcgen::generate_simple_self_signed;
use sha2::{Digest, Sha256};

/// PEM certificate/key pair plus the SHA-256 certificate fingerprint.
#[derive(Clone, Debug)]
pub struct TlsMaterial {
    pub cert_pem: String,
    pub key_pem: String,
    pub fingerprint: String,
}

impl TlsMaterial {
    /// Generates a fresh self-signed certificate for localhost.
    pub fn generate() -> io::Result<Self> {
        let certified =
            generate_simple_self_signed(vec!["localhost".to_owned(), "ferrent.local".to_owned()])
                .map_err(|error| io::Error::other(error.to_string()))?;

        let cert_pem = certified.cert.pem();
        let key_pem = certified.signing_key.serialize_pem();
        let fingerprint = fingerprint(certified.cert.der().as_ref());

        Ok(Self {
            cert_pem,
            key_pem,
            fingerprint,
        })
    }

    /// Loads the certificate/key from `dir`, generating and persisting a pair on
    /// first use. The private key is written with mode 0600 on Unix.
    pub fn load_or_generate(dir: &Path) -> io::Result<Self> {
        let cert_path = dir.join("tls-cert.pem");
        let key_path = dir.join("tls-key.pem");
        let fingerprint_path = dir.join("tls-fingerprint.txt");

        if cert_path.exists() && key_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path)?;
            let key_pem = fs::read_to_string(&key_path)?;
            let fingerprint = if fingerprint_path.exists() {
                fs::read_to_string(&fingerprint_path)?.trim().to_owned()
            } else {
                fingerprint_from_pem(&cert_pem)?
            };
            return Ok(Self {
                cert_pem,
                key_pem,
                fingerprint,
            });
        }

        let material = Self::generate()?;
        fs::create_dir_all(dir)?;
        fs::write(&cert_path, &material.cert_pem)?;
        ferrent_platform::write_secret(&key_path, material.key_pem.as_bytes())?;
        fs::write(&fingerprint_path, &material.fingerprint)?;
        Ok(material)
    }
}

fn fingerprint_from_pem(cert_pem: &str) -> io::Result<String> {
    let mut reader = cert_pem.as_bytes();
    let certificate = rustls_pemfile::certs(&mut reader)
        .next()
        .transpose()?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no certificate in PEM"))?;
    Ok(fingerprint(certificate.as_ref()))
}

fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut output = String::with_capacity(digest.len() * 3);
    for (index, byte) in digest.iter().enumerate() {
        if index > 0 {
            output.push(':');
        }
        write!(output, "{byte:02x}").expect("write to String");
    }
    output
}
