//! Recoverable secret material: authenticated encryption for the credentials
//! integrations need (spec §10.5).
//!
//! What this module guarantees, and what it does not:
//!
//! * **Authenticated encryption with a unique nonce per record.** XChaCha20-
//!   Poly1305 (the `chacha20poly1305` crate, a reviewed implementation — this
//!   module never composes its own construction). The nonce is 24 random bytes
//!   per record, which is why the extended-nonce variant is used: a 192-bit
//!   random nonce does not need a counter to stay unique.
//! * **The row is bound into the ciphertext.** The associated data is
//!   `owner_type|owner_id|name`, so a ciphertext copied to another row fails to
//!   open instead of silently decrypting into somebody else's credential. That
//!   is the point of `a_ciphertext_moved_to_another_row_does_not_open`.
//! * **Rotation is recorded, not improvised.** Every row carries the `key_id`
//!   that encrypted it, and the cipher holds every key it can read, so a retired
//!   key still opens old rows and new writes use the active one.
//! * **A plaintext never reaches a log.** [`Secret`]'s `Debug` prints
//!   `<secret>`; the type has no `Display`, and its only accessor returns the
//!   string explicitly. A stray `?secret` in a log line therefore cannot leak
//!   one.
//! * **Encryption at rest is not a defence against a compromised running
//!   server.** Spec §10.5 requires that limitation to be stated: if an attacker
//!   can run code in this process, they can call [`SecretCipher::decrypt`]. What
//!   this protects is the database at rest, a backup, and a stolen dump.
//!
//! Key material comes from `LOREHAVEN_SECRET_KEY` (base64 or hex, 32 bytes) or
//! from the file named by `[security] secret_key_file`. A development instance
//! with neither generates `<storage_root>/secret.key` and says so in the log; a
//! production instance refuses to start without one, because an ephemeral key
//! would make every stored credential unreadable at the next restart.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

/// The algorithm name stored on every key row and in every record.
pub const ALGORITHM: &str = "xchacha20poly1305";

/// The environment variable that carries the key material.
pub const KEY_ENV: &str = "LOREHAVEN_SECRET_KEY";

/// The file a development instance writes its generated key to.
pub const DEVELOPMENT_KEY_FILE: &str = "secret.key";

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// A plaintext that must not be logged.
///
/// Deliberately not `Display`, deliberately not `Serialize`, and its `Debug`
/// prints a placeholder: the only way to read it is [`Secret::expose`], which is
/// a word a reviewer can see in a diff.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a plaintext.
    #[must_use]
    pub fn new(plaintext: impl Into<String>) -> Self {
        Self(plaintext.into())
    }

    /// The plaintext. Calling this is a decision, and it should be visible.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<secret>")
    }
}

/// One key, with the identifier it is recorded under.
#[derive(Clone)]
pub struct SecretKey {
    key_id: String,
    key: [u8; KEY_LEN],
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The key bytes never appear, not even under `{:?}`.
        formatter
            .debug_struct("SecretKey")
            .field("key_id", &self.key_id)
            .field("key", &"<secret>")
            .finish()
    }
}

impl SecretKey {
    /// A key from raw bytes.
    pub fn from_bytes(key_id: impl Into<String>, bytes: [u8; KEY_LEN]) -> Self {
        Self {
            key_id: key_id.into(),
            key: bytes,
        }
    }

    /// A key parsed from base64 or hex text, which is how it arrives from the
    /// environment or a file.
    ///
    /// Both encodings are accepted because an operator will use whichever one
    /// their tooling produces, and rejecting one of them is a support ticket.
    pub fn parse(key_id: impl Into<String>, text: &str) -> Result<Self> {
        let trimmed = text.trim();
        // Both decoders are tried and the reading that is a key wins. A
        // 64-character hex key is *also* syntactically valid base64 — as 48
        // bytes — so preferring one decoder would reject a key an operator can
        // legitimately write, and the error would blame their input.
        let hexed = hex::decode(trimmed).ok();
        let base64ed = BASE64.decode(trimmed).ok();
        let bytes = [hexed.as_ref(), base64ed.as_ref()]
            .into_iter()
            .flatten()
            .find(|candidate| candidate.len() == KEY_LEN)
            .ok_or_else(|| {
                let seen: Vec<usize> = [hexed.as_ref(), base64ed.as_ref()]
                    .into_iter()
                    .flatten()
                    .map(Vec::len)
                    .collect();
                match seen.as_slice() {
                    [] => anyhow::anyhow!("a secret key is neither base64 nor hex"),
                    lengths => anyhow::anyhow!(
                        "a secret key must be {KEY_LEN} bytes; this text decodes to {lengths:?}"
                    ),
                }
            })?;
        let array: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .expect("the candidate was checked for length");
        Ok(Self::from_bytes(key_id, array))
    }

    /// The identifier recorded on rows this key encrypts.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The key material, hex encoded, for writing to a key file.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.key)
    }
}

/// A record encrypted for one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypted {
    /// Which key encrypted it.
    pub key_id: String,
    /// The nonce, base64.
    pub nonce: String,
    /// The ciphertext, base64.
    pub ciphertext: String,
}

/// The keys an instance can read with, and the one it writes with.
pub struct SecretCipher {
    active: SecretKey,
    /// Every key that may still open a row, by id.
    retired: Vec<SecretKey>,
}

impl fmt::Debug for SecretCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretCipher")
            .field("active_key_id", &self.active.key_id())
            .field("retired_keys", &self.retired.len())
            .finish()
    }
}

impl SecretCipher {
    /// A cipher with one active key and no retired ones.
    #[must_use]
    pub fn new(active: SecretKey) -> Self {
        Self {
            active,
            retired: Vec::new(),
        }
    }

    /// Add a key that may still open old rows.
    #[must_use]
    pub fn with_retired(mut self, key: SecretKey) -> Self {
        self.retired.push(key);
        self
    }

    /// The key new writes use.
    #[must_use]
    pub fn active_key_id(&self) -> &str {
        self.active.key_id()
    }

    fn key_for(&self, key_id: &str) -> Option<&SecretKey> {
        if self.active.key_id() == key_id {
            return Some(&self.active);
        }
        self.retired.iter().find(|key| key.key_id() == key_id)
    }

    /// Encrypt a plaintext for one row.
    ///
    /// `owner_type`, `owner_id` and `name` are authenticated but not encrypted:
    /// they are the record's identity, and binding them means a ciphertext
    /// cannot be moved to another row and still open.
    pub fn encrypt(&self, owner: Record<'_>, plaintext: &Secret) -> Result<Encrypted> {
        let cipher = XChaCha20Poly1305::new((&self.active.key).into());
        let mut nonce_bytes = [0u8; NONCE_LEN];
        // `rand` is already a dependency of the workspace; a nonce that repeats
        // under one key is the one failure mode this construction has.
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.expose().as_bytes(),
                    aad: owner.aad().as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("encrypting a secret failed"))?;
        Ok(Encrypted {
            key_id: self.active.key_id().to_owned(),
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(ciphertext),
        })
    }

    /// Decrypt a record.
    ///
    /// An error here means the ciphertext, the nonce, the associated data or the
    /// key is wrong — which is exactly what it means when a row has been moved,
    /// so the message says so rather than "invalid UTF-8".
    pub fn decrypt(&self, owner: Record<'_>, encrypted: &Encrypted) -> Result<Secret> {
        let key = self
            .key_for(&encrypted.key_id)
            .with_context(|| format!("no key {} is configured", encrypted.key_id))?;
        let nonce = BASE64
            .decode(&encrypted.nonce)
            .context("a stored nonce is not valid base64")?;
        if nonce.len() != NONCE_LEN {
            bail!("a stored nonce is the wrong length");
        }
        let data = BASE64
            .decode(&encrypted.ciphertext)
            .context("a stored ciphertext is not valid base64")?;
        let cipher = XChaCha20Poly1305::new((&key.key).into());
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &data,
                    aad: owner.aad().as_bytes(),
                },
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "a secret did not decrypt: the ciphertext, its row or its key has changed"
                )
            })?;
        let text = String::from_utf8(plaintext).context("a decrypted secret is not UTF-8")?;
        Ok(Secret::new(text))
    }
}

/// Which row a ciphertext belongs to.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// The owning resource type, e.g. `source_credential`.
    pub owner_type: &'a str,
    /// The owning resource id.
    pub owner_id: &'a str,
    /// The name of the secret within the owner.
    pub name: &'a str,
}

impl Record<'_> {
    /// The associated data. A separator that cannot appear in an id keeps the
    /// three fields from being confusable (`a|b` + `c` must not equal `a` +
    /// `b|c`).
    #[must_use]
    fn aad(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.owner_type, self.owner_id, self.name
        )
    }
}

/// Load the instance's key material.
///
/// Order: `LOREHAVEN_SECRET_KEY`, then the configured key file. In development,
/// and only there, a missing key is generated into `<root>/secret.key` so a
/// developer instance works out of the box; in production a missing key is a
/// startup refusal, because an ephemeral key silently makes every stored
/// credential unreadable the next time the process restarts.
pub fn load_cipher(
    storage_root: &Path,
    key_file: Option<&Path>,
    production: bool,
) -> Result<SecretCipher> {
    if let Ok(text) = std::env::var(KEY_ENV) {
        if !text.trim().is_empty() {
            return Ok(SecretCipher::new(SecretKey::parse("env-1", &text)?));
        }
    }

    if let Some(path) = key_file {
        if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            return Ok(SecretCipher::new(SecretKey::parse("file-1", &text)?));
        }
        if production {
            bail!("no secret key: set {KEY_ENV} or create {}", path.display());
        }
    }

    let generated = storage_root.join(DEVELOPMENT_KEY_FILE);
    if generated.exists() {
        let text = std::fs::read_to_string(&generated)
            .with_context(|| format!("reading {}", generated.display()))?;
        return Ok(SecretCipher::new(SecretKey::parse("file-1", &text)?));
    }

    if production {
        bail!("no secret key: set {KEY_ENV}, or name a key file, before starting production");
    }

    // Development only: make one, keep it, and say so.
    let mut bytes = [0u8; KEY_LEN];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let key = SecretKey::from_bytes("file-1", bytes);
    std::fs::create_dir_all(storage_root)
        .with_context(|| format!("creating {}", storage_root.display()))?;
    write_private(&generated, &key.to_hex())?;
    tracing::warn!(
        path = %generated.display(),
        "no secret key was configured; generated one for development. Back it up or set {} \
         before this instance holds anything you care about",
        KEY_ENV
    );
    Ok(SecretCipher::new(key))
}

/// Write a file that only its owner can read.
fn write_private(path: &PathBuf, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// Store a secret, returning the row id that names it.
///
/// The plaintext is encrypted here and then never held again: the caller gets an
/// id, and reading the value back is a separate act that [`open_secret`]
/// performs. Keeping those two acts apart is what stops a route handler from
/// holding a credential it did not mean to hold, and therefore from logging one.
///
/// # Errors
/// Returns an error if the key cannot encrypt, or the row cannot be written.
pub async fn seal_secret(
    db: &lorehaven_db::Database,
    cipher: &SecretCipher,
    owner_type: &str,
    owner_id: &str,
    name: &str,
    plaintext: &Secret,
) -> anyhow::Result<String> {
    let owner = Record {
        owner_type,
        owner_id,
        name,
    };
    let sealed = cipher.encrypt(owner, plaintext)?;
    // The key must be a row before a secret can name it: `secrets.key_id` is a
    // foreign key, and a key the table has never heard of is a key this database
    // will not let anything be encrypted with.
    lorehaven_db::secrets::ensure_encryption_key(db, &sealed.key_id, "XChaCha20-Poly1305").await?;
    lorehaven_db::secrets::put_secret(
        db,
        owner_type,
        owner_id,
        name,
        &sealed.key_id,
        &sealed.nonce,
        &sealed.ciphertext,
    )
    .await
}

/// Read a stored secret back, given the row that names it.
///
/// `None` means the row is gone, which a caller should treat as a credential it
/// no longer has rather than as a decryption failure: those two need different
/// messages, because only one of them is a bug.
///
/// # Errors
/// Returns an error if the row exists and cannot be decrypted — a retired key
/// that has been dropped, or a row written by a different instance.
pub async fn open_secret(
    db: &lorehaven_db::Database,
    cipher: &SecretCipher,
    secret_id: &str,
) -> anyhow::Result<Option<String>> {
    let Some(row) = lorehaven_db::secrets::get_secret_by_id(db, secret_id).await? else {
        return Ok(None);
    };
    let owner = Record {
        owner_type: &row.owner_type,
        owner_id: &row.owner_id,
        name: &row.name,
    };
    let sealed = Encrypted {
        key_id: row.key_id,
        nonce: row.nonce,
        ciphertext: row.ciphertext,
    };
    let opened = cipher.decrypt(owner, &sealed)?;
    Ok(Some(opened.expose().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key whose id is derived from its material, the way a real rotation
    /// works: a new key is a new `key_id`, never the old id over new bytes.
    fn key(byte: u8) -> SecretKey {
        SecretKey::from_bytes(format!("test-{byte}"), [byte; KEY_LEN])
    }

    fn record() -> Record<'static> {
        Record {
            owner_type: "source_credential",
            owner_id: "11111111-1111-1111-1111-111111111111",
            name: "archiveofourown",
        }
    }

    #[test]
    fn a_secret_round_trips() {
        let cipher = SecretCipher::new(key(7));
        let plaintext = Secret::new("a source password");
        let encrypted = cipher.encrypt(record(), &plaintext).expect("encrypt");
        let decrypted = cipher.decrypt(record(), &encrypted).expect("decrypt");
        assert_eq!(decrypted, plaintext);
        // The plaintext is not recoverable from the stored form without the key.
        assert!(!encrypted.ciphertext.contains("password"));
    }

    #[test]
    fn a_ciphertext_moved_to_another_row_does_not_open() {
        let cipher = SecretCipher::new(key(7));
        let encrypted = cipher
            .encrypt(record(), &Secret::new("a source password"))
            .expect("encrypt");

        // Same bytes, different row: the associated data no longer matches.
        let moved = Record {
            owner_id: "22222222-2222-2222-2222-222222222222",
            ..record()
        };
        assert!(
            cipher.decrypt(moved, &encrypted).is_err(),
            "a secret moved to another row must fail to open"
        );

        let renamed = Record {
            name: "another-source",
            ..record()
        };
        assert!(cipher.decrypt(renamed, &encrypted).is_err());
    }

    #[test]
    fn a_nonce_is_never_reused_for_one_plaintext() {
        let cipher = SecretCipher::new(key(7));
        let first = cipher
            .encrypt(record(), &Secret::new("same"))
            .expect("first");
        let second = cipher
            .encrypt(record(), &Secret::new("same"))
            .expect("second");
        assert_ne!(
            first.nonce, second.nonce,
            "a repeated nonce under one key breaks the guarantee the AEAD rests on"
        );
        assert_ne!(first.ciphertext, second.ciphertext);
    }

    #[test]
    fn a_retired_key_still_opens_what_it_encrypted() {
        let old = key(1);
        let encrypted = SecretCipher::new(old.clone())
            .encrypt(record(), &Secret::new("rotating"))
            .expect("encrypt");

        // After a rotation the old key is kept for reading, and new writes use
        // the new one.
        let rotated = SecretCipher::new(key(2)).with_retired(old);
        assert_eq!(
            rotated
                .decrypt(record(), &encrypted)
                .expect("decrypt")
                .expose(),
            "rotating"
        );
        let fresh = rotated
            .encrypt(record(), &Secret::new("after rotation"))
            .expect("encrypt");
        assert_eq!(fresh.key_id, rotated.active_key_id());
        assert_ne!(fresh.key_id, encrypted.key_id);
    }

    #[test]
    fn a_ciphertext_under_an_unknown_key_is_refused() {
        let encrypted = SecretCipher::new(key(1))
            .encrypt(record(), &Secret::new("gone"))
            .expect("encrypt");
        let cipher = SecretCipher::new(key(2));
        let error = cipher
            .decrypt(record(), &encrypted)
            .expect_err("must refuse");
        assert!(error.to_string().contains("no key"), "{error}");
    }

    /// The point of the wrapper type: a `?secret` in a log line prints a
    /// placeholder. Without `Debug` printing `<secret>`, this is the test that
    /// cannot be written and the leak that cannot be spotted.
    #[test]
    fn a_secret_is_not_in_the_logs() {
        let secret = Secret::new("hunter2-the-source-password");
        let logged = format!("{secret:?}");
        assert_eq!(logged, "<secret>");
        assert!(!logged.contains("hunter2"));
        // A whole record, as a log line or a debug dump would render it.
        let encrypted = SecretCipher::new(key(9))
            .encrypt(record(), &secret)
            .expect("encrypt");
        assert!(!format!("{encrypted:?}").contains("hunter2"));
        assert!(
            !format!("{:?}", SecretCipher::new(key(9)).with_retired(key(3)))
                .contains(&hex::encode([9u8; KEY_LEN]))
        );
    }

    #[test]
    fn a_key_parses_from_base64_and_hex() {
        let raw = [42u8; KEY_LEN];
        let from_hex = SecretKey::parse("hex-1", &hex::encode(raw)).expect("hex");
        let from_base64 = SecretKey::parse("b64-1", &BASE64.encode(raw)).expect("base64");
        assert_eq!(from_hex.key, raw);
        assert_eq!(from_base64.key, raw);
        assert_eq!(from_hex.key_id(), "hex-1");
        assert!(SecretKey::parse("short", "abc").is_err());
        assert!(SecretKey::parse("junk", "not a key at all!!").is_err());
    }
}
