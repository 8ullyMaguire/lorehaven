//! Build identity.
//!
//! Spec §22 and §23 make deployment and incident work first-class: the first
//! question after any deploy is "which build is actually running?". The build
//! script compiles the git revision into the binary so `/health/live` and
//! `/api/v1/meta` can answer without shell access.

/// Crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git revision the binary was built from.
pub const GIT_REVISION: &str = env!("LOREHAVEN_GIT_REVISION");

/// Whether the working tree had uncommitted changes at build time.
pub const GIT_DIRTY: &str = env!("LOREHAVEN_GIT_DIRTY");

/// Unix timestamp of the build, as text.
pub const BUILD_UNIX: &str = env!("LOREHAVEN_BUILD_TIME");

/// Human-readable build identifier, e.g. `0.1.0+11f0bb8` or `0.1.0+11f0bb8.dirty`.
#[must_use]
pub fn build_id() -> String {
    if GIT_DIRTY == "dirty" {
        format!("{VERSION}+{GIT_REVISION}.dirty")
    } else {
        format!("{VERSION}+{GIT_REVISION}")
    }
}

/// The API version served under `/api/v1`.
pub const API_VERSION: &str = "v1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_id_identifies_version_and_revision() {
        let id = build_id();
        assert!(id.starts_with(VERSION), "got {id}");
        assert!(id.contains('+'), "got {id}");
    }

    #[test]
    fn the_build_timestamp_is_numeric() {
        assert!(
            BUILD_UNIX.parse::<u64>().is_ok(),
            "build script should have written a unix timestamp, got {BUILD_UNIX:?}"
        );
    }
}
