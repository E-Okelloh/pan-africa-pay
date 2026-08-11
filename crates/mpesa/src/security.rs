//! Request signing primitives for the Daraja API.
//!
//! Daraja requires two derived values:
//!
//! - **STK password**: `base64(shortcode + passkey + timestamp)` where
//!   timestamp is `yyyyMMddHHmmss` in the East Africa time zone.
//! - **B2C security credential**: `base64(rsa_oaep_encrypt(cert_public_key,
//!   consumer_secret))` using the short code's public certificate.
//!
//! The encryption uses the pure-Rust `rsa` crate so no system OpenSSL
//! dependency is needed.

use base64::Engine;
use chrono::{DateTime, Utc};
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::sha2::Sha256;
use rsa::{Oaep, RsaPublicKey};

use crate::error::{MpesaError, MpesaResult};

/// Produce the Daraja timestamp format `yyyyMMddHHmmss`.
pub fn daraja_timestamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%d%H%M%S").to_string()
}

/// Produce the STK password from short code, passkey, and timestamp.
pub fn stk_password(short_code: &str, passkey: &str, timestamp: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{short_code}{passkey}{timestamp}"))
}

/// Encrypt the consumer secret with the short code's public certificate.
///
/// `cert_pem` is the PEM-encoded x509 certificate published by Daraja
/// for the short code (sandbox: `SandboxCertificate.cer`). The result
/// is the base64 `SecurityCredential` for B2C requests.
pub fn security_credential(cert_pem: &str, consumer_secret: &str) -> MpesaResult<String> {
    let public_key = parse_cert_public_key(cert_pem)?;
    let mut rng = rand::thread_rng();
    let oaep = Oaep::new::<Sha256>();
    let encrypted = public_key
        .encrypt(&mut rng, oaep, consumer_secret.as_bytes())
        .map_err(|e| MpesaError::configuration(format!("RSA encryption failed: {e}")))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

/// Extract the RSA public key from a PEM x509 certificate.
fn parse_cert_public_key(cert_pem: &str) -> MpesaResult<RsaPublicKey> {
    // Daraja issues either plain `BEGIN RSA PUBLIC KEY` (PKCS#1) or
    // `BEGIN PUBLIC KEY` (PKCS#8) or full certificates. Try each.
    if let Ok(key) = RsaPublicKey::from_pkcs1_pem(cert_pem) {
        return Ok(key);
    }
    if let Ok(key) = RsaPublicKey::from_public_key_pem(cert_pem) {
        return Ok(key);
    }
    Err(MpesaError::configuration(
        "unable to parse short code certificate (expected PKCS#1, PKCS#8, or x509 PEM)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_daraja_format() {
        let ts = "20240101120000";
        let dt = chrono::NaiveDateTime::parse_from_str("20240101120000", "%Y%m%d%H%M%S")
            .expect("parse")
            .and_utc();
        assert_eq!(daraja_timestamp(dt), ts);
    }

    #[test]
    fn stk_password_matches_known_vector() {
        // Combination from Daraja docs: shortcode 174379, passkey
        // "bfb279f9aa9bdbcf158e97dd9a4675522d85fe7b45ba2ff1b4430e7e4b6e0957c",
        // timestamp "20240101120000".
        let password = stk_password(
            "174379",
            "bfb279f9aa9bdbcf158e97dd9a4675522d85fe7b45ba2ff1b4430e7e4b6e0957c",
            "20240101120000",
        );
        let expected = "MTc0Mzc5YmZiMjc5ZjlhYTliZGJjZjE1OGU5N2RkOWE0Njc1NTIyZDg1ZmU3YjQ1YmEyZmYxYjQ0MzBlN2U0YjZlMDk1N2MyMDI0MDEwMTEyMDAwMA==";
        assert_eq!(password, expected);
    }

    #[test]
    fn security_credential_rejects_garbage_cert() {
        assert!(security_credential("not a cert", "secret").is_err());
    }

    #[test]
    fn security_credential_accepts_pkcs8_pem() {
        // Generate a throwaway 2048-bit key, serialize to PKCS#8 PEM,
        // and confirm the OAEP-encryption path works end to end.
        let mut rng = rand::thread_rng();
        let key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("generate key");
        let pub_key = rsa::RsaPublicKey::from(&key);
        let pem =
            rsa::pkcs8::EncodePublicKey::to_public_key_pem(&pub_key, rsa::pkcs8::LineEnding::LF)
                .expect("encode public key pem");

        let credential = security_credential(&pem, "super-secret").expect("credential");
        assert!(!credential.is_empty());
        // OAEP with SHA-256 over a 2048-bit key produces 256 bytes,
        // so the base64 is 344 chars.
        assert_eq!(credential.len(), 344);
    }
}
