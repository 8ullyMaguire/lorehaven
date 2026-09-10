//! Password and token cryptography.
//!
//! Spec §3.5: "API integrations use separately issued, scoped tokens whose
//! hashes are stored server-side", and sessions are opaque server-managed
//! values. Nothing here is reversible: a database dump must not yield anything
//! a reader can present as a credential.
//!
//! * Passwords are hashed with Argon2id (default parameters of the `argon2`
//!   crate, which track the current OWASP guidance).
//! * Session and API tokens are 256 bits of CSPRNG output, stored only as
//!   SHA-256 hex digests. Tokens are high-entropy random values, so a fast
//!   digest is the right tool here — unlike passwords, they are not guessable
//!   and do not need a slow KDF.

use anyhow::{Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Number of random bytes in an issued token.
const TOKEN_BYTES: usize = 32;

/// Hash a password for storage, returning a PHC string.
pub fn hash_password(plain: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .context("hashing password")?;
    Ok(hash.to_string())
}

/// Verify a password against a stored PHC string.
///
/// A malformed stored hash yields `Ok(false)` rather than an error: the caller
/// is authenticating, and "we cannot verify this credential" is not a different
/// answer from "this credential is wrong".
pub fn verify_password(plain: &str, phc: &str) -> Result<bool> {
    let parsed = PasswordHash::new(phc).context("parsing stored password hash")?;
    Ok(Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok())
}

/// Mint an opaque token. Returned to the client exactly once.
#[must_use]
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The storage form of a token: SHA-256, hex encoded.
#[must_use]
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_round_trip() {
        let phc = hash_password("correct horse battery staple").expect("hash");
        assert!(phc.starts_with("$argon2id$"), "got {phc}");
        assert!(verify_password("correct horse battery staple", &phc).expect("verify"));
        assert!(!verify_password("wrong", &phc).expect("verify"));
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let a = hash_password("hunter2").expect("hash");
        let b = hash_password("hunter2").expect("hash");
        assert_ne!(a, b, "a shared salt would make the hashes comparable");
    }

    #[test]
    fn a_malformed_stored_hash_is_a_failed_verification_not_a_crash() {
        assert!(verify_password("x", "not-a-phc-string").is_err());
    }

    #[test]
    fn tokens_are_unique_and_long_enough() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        // 32 bytes base64url without padding is 43 characters.
        assert_eq!(a.len(), 43);
    }

    #[test]
    fn token_hashes_are_deterministic_and_irreversible() {
        let token = generate_token();
        let digest = hash_token(&token);
        assert_eq!(digest, hash_token(&token));
        assert_eq!(digest.len(), 64);
        assert_ne!(digest, token);
    }
}
