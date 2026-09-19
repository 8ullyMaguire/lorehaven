//! M16 — Webhook domain: event envelope, HMAC signing, bounded payload.

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Webhook event envelope.
#[derive(Debug, Clone)]
pub struct WebhookEvent {
    pub event_type: String,
    pub event_id: String,
    pub created_at: String,
    pub payload: serde_json::Value,
}

impl WebhookEvent {
    /// Serialize the envelope for signing.
    pub fn canonical_string(&self) -> String {
        format!("{}:{}:{}", self.event_type, self.event_id, self.created_at)
    }

    /// Compute HMAC-SHA256 signature over the canonical string + payload.
    ///
    /// Uses proper HMAC-SHA256 (RFC 2104) with a 32-byte output, encoded as hex.
    /// The signing input is `canonical_string + payload.to_string()`.
    pub fn sign(&self, secret: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .expect("HMAC can take a key of any size");
        mac.update(self.canonical_string().as_bytes());
        mac.update(self.payload.to_string().as_bytes());
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Verify a signature against the envelope.
    pub fn verify(&self, secret: &str, signature: &str) -> bool {
        self.sign(secret) == signature
    }
}

/// Bound a payload to a maximum size (in bytes). Returns the original if within bounds,
/// or a truncated version with a marker if over.
pub fn bound_payload(payload: &serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let serialized = payload.to_string();
    if serialized.len() <= max_bytes {
        payload.clone()
    } else {
        serde_json::json!({
            "_truncated": true,
            "_original_bytes": serialized.len(),
            "_max_bytes": max_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_sign_and_verify() {
        let event = WebhookEvent {
            event_type: "work.published".to_string(),
            event_id: "evt-123".to_string(),
            created_at: "2026-09-14T12:00:00Z".to_string(),
            payload: serde_json::json!({"work_id": "w-1"}),
        };

        let secret = "whsec_test";
        let sig = event.sign(secret);
        assert!(event.verify(secret, &sig));
        assert!(!event.verify("wrong-secret", &sig));
    }

    /// RFC 4231 test case 2: key = "Jefe", data = "what do ya want for nothing?".
    /// Tested directly since `sign()` prepends the event envelope.
    #[test]
    fn hmac_matches_rfc4231_test_case_2() {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        let result = mac.finalize();
        assert_eq!(
            hex::encode(result.into_bytes()),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn payload_within_bounds() {
        let payload = serde_json::json!({"key": "value"});
        let bounded = bound_payload(&payload, 1000);
        assert_eq!(bounded, payload);
    }

    #[test]
    fn payload_truncated() {
        let payload = serde_json::json!({"large": "x".repeat(1000)});
        let bounded = bound_payload(&payload, 100);
        assert!(bounded["_truncated"].as_bool().unwrap());
    }
}
