use std::path::PathBuf;

/// Load the ActivityPub public key.
///
/// Reads from `~/.config/lorehaven/federation.pub` at runtime, falling back to
/// the bundled key if the file is missing. The private key (needed for signing
/// outgoing activities) is read from `~/.config/lorehaven/federation.pem` by
/// the HTTP layer when signing requests.
pub fn load_public_key_pem() -> String {
    if let Ok(pem) = std::fs::read_to_string(public_key_path()) {
        return pem;
    }
    // Fallback: bundled public key.
    include_str!("federation.pub").to_string()
}

/// Path to the instance's federation public key.
pub fn public_key_path() -> PathBuf {
    config_dir().join("federation.pub")
}

/// Path to the instance's federation private key.
pub fn private_key_path() -> PathBuf {
    config_dir().join("federation.pem")
}

fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home/alvaro/"))
        .join(".config/lorehaven")
}

/// Generate a fresh RSA keypair into the config directory.
///
/// Called by `lorehaven generate-federation-key`. Idempotent: does nothing if a
/// key already exists.
pub fn generate_keypair() -> anyhow::Result<()> {
    let priv_path = private_key_path();
    if priv_path.exists() {
        return Ok(());
    }
    if let Some(parent) = priv_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = std::process::Command::new("openssl")
        .args(["genrsa", "-out", priv_path.to_str().unwrap(), "2048"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("openssl genrsa failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let pub_path = public_key_path();
    let output = std::process::Command::new("openssl")
        .args(["rsa", "-in", priv_path.to_str().unwrap(), "-pubout"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("openssl rsa pubout failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    std::fs::write(&pub_path, &output.stdout)?;
    tracing::info!(path = %priv_path.display(), "generated ActivityPub RSA keypair");
    Ok(())
}
