//! TLS certificate fingerprints used for client-side pinning.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// Computes the SHA-256 fingerprint of the first certificate in `pem`.
pub fn fingerprint_from_pem(pem: &str) -> Result<String, String> {
    let mut reader = pem.as_bytes();
    let certificate = rustls_pemfile::certs(&mut reader)
        .next()
        .ok_or_else(|| "no certificate in PEM".to_owned())?
        .map_err(|error| error.to_string())?;
    Ok(fingerprint(certificate.as_ref()))
}

/// Formats a DER certificate's SHA-256 as colon-separated lowercase hex.
#[must_use]
pub fn fingerprint(der: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::fingerprint;

    #[test]
    fn sha256_is_formatted_with_colons() {
        assert_eq!(
            fingerprint(b"abc"),
            "ba:78:16:bf:8f:01:cf:ea:41:41:40:de:5d:ae:22:23:b0:03:61:a3:96:17:7a:9c:b4:10:ff:61:f2:00:15:ad"
        );
    }
}
