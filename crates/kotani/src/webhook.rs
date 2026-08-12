//! Webhook signature verification.
//!
//! When a webhook secret is configured, Kotani Pay signs every callback
//! with an HMAC-SHA256 of the raw request body, delivered in the
//! `X-Kotani-Signature` header. We compare using a constant-time
//! comparison to avoid timing attacks.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify an `X-Kotani-Signature` header against the raw request body.
///
/// Returns false when the header is missing, malformed, or does not
/// match (constant-time comparison).
pub fn verify_signature(secret: &str, raw_body: &[u8], signature: &str) -> bool {
    if secret.is_empty() || signature.is_empty() {
        return false;
    }
    let decoded = match hex::decode(signature.trim()) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(raw_body);
    mac.verify_slice(&decoded).is_ok()
}

/// Hex signature helper for tests and tooling.
pub fn sign(secret: &str, raw_body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("valid key");
    mac.update(raw_body);
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-webhook-secret";

    #[test]
    fn valid_signature_passes() {
        let body = br#"{"event":"deposit.success"}"#;
        let sig = sign(SECRET, body);
        assert!(verify_signature(SECRET, body, &sig));
    }

    #[test]
    fn tampered_body_fails() {
        let body = br#"{"event":"deposit.success"}"#;
        let sig = sign(SECRET, body);
        assert!(!verify_signature(
            SECRET,
            b"{\"event\":\"deposit.failed\"}",
            &sig
        ));
    }

    #[test]
    fn wrong_secret_fails() {
        let body = br#"{"event":"deposit.success"}"#;
        let sig = sign("other-secret", body);
        assert!(!verify_signature(SECRET, body, &sig));
    }

    #[test]
    fn missing_or_garbage_signature_fails() {
        let body = br#"{"event":"deposit.success"}"#;
        assert!(!verify_signature(SECRET, body, ""));
        assert!(!verify_signature(SECRET, body, "not-hex!"));
        assert!(!verify_signature("", body, "abcd"));
    }
}
