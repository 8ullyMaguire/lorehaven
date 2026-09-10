//! Configuration loading.
//!
//! Documented precedence (spec §5):
//!
//! ```text
//! command-line argument  →  environment variable  →  configuration file  →  default
//! ```
//!
//! Every value is resolved the same way, in one place, so `doctor` and `serve`
//! can never disagree about what the running configuration is.
//!
//! The file is parsed with `deny_unknown_fields`: a typo in a self-hosted
//! operator's `lorehaven.toml` is a startup error with a precise message,
//! not a silently ignored line.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use lorehaven_db::DatabaseConfig;
use serde::Deserialize;

use crate::cli::GlobalArgs;

/// The default configuration file name, looked up in the working directory.
pub const DEFAULT_CONFIG_FILE: &str = "lorehaven.toml";

/// Runtime environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Local development: permissive, verbose.
    Development,
    /// Automated tests: isolated, quiet.
    Test,
    /// A real deployment: strict.
    Production,
}

impl Environment {
    /// Parse the environment name.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" | "prod" => Ok(Self::Production),
            other => anyhow::bail!(
                "unknown environment {other:?}; expected development, test or production"
            ),
        }
    }

    /// Stable lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        }
    }

    /// Whether this is a production deployment.
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, for a terminal.
    Pretty,
    /// One JSON object per line, for a log collector.
    Json,
}

impl LogFormat {
    /// Parse the format name.
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pretty" | "text" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            other => anyhow::bail!("unknown log format {other:?}; expected pretty or json"),
        }
    }
}

/// The resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Active environment.
    pub environment: Environment,
    /// Public identity of the instance.
    pub site: SiteConfig,
    /// HTTP listener settings.
    pub server: ServerConfig,
    /// Database settings.
    pub database: DatabaseConfig,
    /// File storage settings.
    pub storage: StorageConfig,
    /// Cookie and session settings.
    pub security: SecurityConfig,
    /// Logging settings.
    pub logging: LoggingConfig,
    /// Static asset settings.
    pub assets: AssetsConfig,
    /// Development-only affordances.
    pub dev: DevConfig,
    /// Account and registration settings.
    pub accounts: AccountsConfig,
    /// Age-policy settings.
    pub age: AgeConfig,
    /// Rate limits.
    pub rate_limits: crate::limiter::Limits,
    /// Where the configuration file was read from, if any.
    pub config_path: Option<PathBuf>,
}

/// Account creation settings.
#[derive(Debug, Clone)]
pub struct AccountsConfig {
    /// Whether new accounts may register at all.
    ///
    /// Exposed because a small instance frequently closes registration after an
    /// initial cohort; making that a configuration value rather than a code
    /// change keeps it an operational decision.
    pub registration_open: bool,
}

/// Age-policy settings.
///
/// Spec §7 requires the age machinery to be *configured*, not hard-coded to a
/// jurisdiction, and warns explicitly against allowing unrestricted child
/// registration merely because a checkbox exists.
#[derive(Debug, Clone)]
pub struct AgeConfig {
    /// The age at which a person may consent to their own data processing.
    /// Fourteen under Spanish law, which is the operator's stated basis.
    pub threshold: u8,
    /// Whether an under-threshold authorization workflow is actually in place.
    ///
    /// **False by default.** While it is false, an account that declares itself
    /// under the threshold is created in the `restricted` state: it may read,
    /// and it may not write, message or be discovered. Turning this on is a
    /// legal and operational decision, not a feature toggle.
    pub guardian_workflow_enabled: bool,
}

/// Public identity of the instance.
#[derive(Debug, Clone)]
pub struct SiteConfig {
    /// Human-readable site name.
    pub name: String,
    /// Public base URL, without a trailing slash.
    pub base_url: String,
    /// Operator contact address, for legal pages and feeds.
    pub contact_email: Option<String>,
}

/// HTTP listener settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Interface to bind.
    pub bind: String,
    /// Port to bind.
    pub port: u16,
    /// Maximum accepted request body size.
    pub max_body_bytes: usize,
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Allowed CORS origins. Empty means same-origin only.
    pub cors_origins: Vec<String>,
}

/// File storage settings.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Root directory for all managed files.
    pub root: PathBuf,
}

/// Cookie and session settings.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Whether cookies carry the `Secure` attribute.
    pub cookie_secure: bool,
    /// Session lifetime.
    pub session_ttl: Duration,
    /// Whether state-changing cookie-authenticated requests need a CSRF token.
    pub csrf_required: bool,
    /// Whether `X-Forwarded-For` may be believed for rate-limit keying.
    ///
    /// Off by default, and it must stay off unless a reverse proxy is genuinely
    /// in front: the header is trivially forgeable, and trusting it lets a
    /// client mint a fresh rate-limit bucket per request.
    pub trust_proxy: bool,
}

/// Logging settings.
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Tracing filter directive.
    pub filter: String,
    /// Output format.
    pub format: LogFormat,
}

/// Static asset settings.
#[derive(Debug, Clone)]
pub struct AssetsConfig {
    /// Serve assets from this directory instead of the embedded bundle.
    /// Development only: a production deployment serves what was compiled in.
    pub dir: Option<PathBuf>,
}

/// Development-only affordances.
#[derive(Debug, Clone)]
pub struct DevConfig {
    /// Whether the seed command may run against this instance.
    pub seed_enabled: bool,
}

impl Config {
    /// Resolve the configuration from arguments, environment, file and defaults.
    pub fn load(global: &GlobalArgs) -> Result<Self> {
        let config_path = match &global.config {
            Some(path) => {
                if !path.exists() {
                    anyhow::bail!(
                        "configuration file {} does not exist (from --config)",
                        path.display()
                    );
                }
                Some(path.clone())
            }
            None => {
                let default = PathBuf::from(DEFAULT_CONFIG_FILE);
                default.exists().then_some(default)
            }
        };

        let file: FileConfig = match &config_path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
            }
            None => FileConfig::default(),
        };

        let environment = match &global.environment {
            Some(value) => Environment::parse(value)?,
            None => match &file.environment {
                Some(value) => Environment::parse(value)?,
                None => Environment::Development,
            },
        };

        let production = environment.is_production();

        // --- site -----------------------------------------------------------
        let site_file = file.site.unwrap_or_default();
        let base_url = global
            .base_url
            .clone()
            .or(site_file.base_url)
            .unwrap_or_else(|| {
                let port = global.port.or(file.server.as_ref().and_then(|s| s.port));
                match port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "http://localhost:8080".to_owned(),
                }
            });

        let site = SiteConfig {
            name: site_file.name.unwrap_or_else(|| "Lorehaven".to_owned()),
            base_url: base_url.trim_end_matches('/').to_owned(),
            contact_email: site_file.contact_email,
        };

        // --- server ---------------------------------------------------------
        let server_file = file.server.unwrap_or_default();
        let server = ServerConfig {
            bind: global
                .bind
                .clone()
                .or(server_file.bind)
                .unwrap_or_else(|| "127.0.0.1".to_owned()),
            port: global.port.or(server_file.port).unwrap_or(8080),
            max_body_bytes: server_file.max_body_bytes.unwrap_or(2 * 1024 * 1024),
            request_timeout: Duration::from_secs(server_file.request_timeout_secs.unwrap_or(30)),
            cors_origins: server_file.cors_origins.unwrap_or_default(),
        };

        // --- storage --------------------------------------------------------
        let storage_file = file.storage.unwrap_or_default();
        let storage_root = global
            .storage_root
            .clone()
            .or(storage_file.root)
            .unwrap_or_else(|| {
                if production {
                    PathBuf::from("/var/lib/lorehaven")
                } else {
                    PathBuf::from("./data")
                }
            });

        let storage = StorageConfig {
            root: storage_root.clone(),
        };

        // --- database -------------------------------------------------------
        let database_file = file.database.unwrap_or_default();
        let database_url = global
            .database_url
            .clone()
            .or(database_file.url)
            .unwrap_or_else(|| {
                format!(
                    "sqlite://{}/lorehaven.sqlite?mode=rwc",
                    storage_root.display()
                )
            });

        let mut database = DatabaseConfig::new(database_url);
        database.max_connections =
            database_file
                .max_connections
                .unwrap_or(if production { 10 } else { 5 });
        if let Some(secs) = database_file.acquire_timeout_secs {
            database.acquire_timeout = Duration::from_secs(secs);
        }

        // --- security -------------------------------------------------------
        let security_file = file.security.unwrap_or_default();
        let security = SecurityConfig {
            // Secure cookies are the production default; development on
            // http://localhost would silently break with them on.
            cookie_secure: security_file.cookie_secure.unwrap_or(production),
            session_ttl: Duration::from_secs(
                60 * 60 * 24 * u64::from(security_file.session_ttl_days.unwrap_or(30)),
            ),
            csrf_required: security_file.csrf_required.unwrap_or(true),
            trust_proxy: security_file.trust_proxy.unwrap_or(false),
        };

        // --- logging --------------------------------------------------------
        let logging_file = file.logging.unwrap_or_default();
        let logging = LoggingConfig {
            filter: global
                .log
                .clone()
                .or(logging_file.filter)
                .unwrap_or_else(|| {
                    if production {
                        "info".to_owned()
                    } else {
                        "info,lorehaven_app=debug,lorehaven_db=debug".to_owned()
                    }
                }),
            format: match &global.log_format {
                Some(value) => LogFormat::parse(value)?,
                None => match &logging_file.format {
                    Some(value) => LogFormat::parse(value)?,
                    None if production => LogFormat::Json,
                    None => LogFormat::Pretty,
                },
            },
        };

        // --- assets ---------------------------------------------------------
        let assets_file = file.assets.unwrap_or_default();
        let assets = AssetsConfig {
            dir: assets_file.dir,
        };

        // --- development ----------------------------------------------------
        let dev_file = file.dev.unwrap_or_default();
        let dev = DevConfig {
            seed_enabled: dev_file.seed_enabled.unwrap_or(!production),
        };

        // --- accounts -------------------------------------------------------
        let accounts_file = file.accounts.unwrap_or_default();
        let accounts = AccountsConfig {
            registration_open: accounts_file.registration_open.unwrap_or(true),
        };

        // --- age policy -----------------------------------------------------
        let age_file = file.age.unwrap_or_default();
        let age = AgeConfig {
            threshold: age_file.threshold.unwrap_or(14),
            guardian_workflow_enabled: age_file.guardian_workflow_enabled.unwrap_or(false),
        };

        // --- rate limits ----------------------------------------------------
        let rate_limits = {
            let defaults = crate::limiter::Limits::default();
            let section = file.rate_limits.unwrap_or_default();
            let build =
                |burst: Option<u32>, per_minute: Option<u32>, fallback: crate::limiter::Quota| {
                    crate::limiter::Quota {
                        burst: burst.unwrap_or(fallback.burst),
                        per_minute: per_minute.unwrap_or(fallback.per_minute),
                    }
                };
            crate::limiter::Limits {
                auth: build(section.auth_burst, section.auth_per_minute, defaults.auth),
                write: build(
                    section.write_burst,
                    section.write_per_minute,
                    defaults.write,
                ),
                search: build(
                    section.search_burst,
                    section.search_per_minute,
                    defaults.search,
                ),
                default: build(
                    section.default_burst,
                    section.default_per_minute,
                    defaults.default,
                ),
                address_multiplier: section
                    .address_multiplier
                    .unwrap_or(defaults.address_multiplier),
            }
        };

        let config = Self {
            environment,
            site,
            server,
            database,
            storage,
            security,
            logging,
            assets,
            dev,
            accounts,
            age,
            rate_limits,
            config_path,
        };

        config.validate()?;
        Ok(config)
    }

    /// Build a configuration for tests and embedded use, bypassing the file.
    #[must_use]
    pub fn development_defaults() -> Self {
        Self {
            environment: Environment::Development,
            site: SiteConfig {
                name: "Lorehaven".to_owned(),
                base_url: "http://localhost:8080".to_owned(),
                contact_email: None,
            },
            server: ServerConfig {
                bind: "127.0.0.1".to_owned(),
                port: 8080,
                max_body_bytes: 2 * 1024 * 1024,
                request_timeout: Duration::from_secs(30),
                cors_origins: Vec::new(),
            },
            database: DatabaseConfig::new("sqlite://:memory:"),
            storage: StorageConfig {
                root: PathBuf::from("./data"),
            },
            security: SecurityConfig {
                cookie_secure: false,
                session_ttl: Duration::from_secs(60 * 60 * 24 * 30),
                csrf_required: true,
                trust_proxy: false,
            },
            logging: LoggingConfig {
                filter: "info".to_owned(),
                format: LogFormat::Pretty,
            },
            assets: AssetsConfig { dir: None },
            dev: DevConfig { seed_enabled: true },
            accounts: AccountsConfig {
                registration_open: true,
            },
            age: AgeConfig {
                threshold: 14,
                guardian_workflow_enabled: false,
            },
            rate_limits: crate::limiter::Limits::default(),
            config_path: None,
        }
    }

    /// Reject values that cannot be sane in any environment.
    ///
    /// Environment-specific rules (a production instance must not run with
    /// development settings) live in [`crate::safety`].
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            anyhow::bail!("server.port must not be 0");
        }
        if !(1024..=64 * 1024 * 1024).contains(&self.server.max_body_bytes) {
            anyhow::bail!(
                "server.max_body_bytes must be between 1024 and 67108864, got {}",
                self.server.max_body_bytes
            );
        }
        let timeout = self.server.request_timeout.as_secs();
        if !(1..=600).contains(&timeout) {
            anyhow::bail!("server.request_timeout_secs must be between 1 and 600, got {timeout}");
        }
        if self.security.session_ttl.as_secs() == 0 {
            anyhow::bail!("security.session_ttl_days must be greater than zero");
        }
        if !self.site.base_url.starts_with("http://") && !self.site.base_url.starts_with("https://")
        {
            anyhow::bail!(
                "site.base_url must start with http:// or https://, got {:?}",
                self.site.base_url
            );
        }
        if self.site.name.trim().is_empty() {
            anyhow::bail!("site.name must not be empty");
        }
        if !(13..=18).contains(&self.age.threshold) {
            anyhow::bail!(
                "age.threshold must be between 13 and 18, got {}",
                self.age.threshold
            );
        }
        if self.rate_limits.write.burst == 0 || self.rate_limits.auth.burst == 0 {
            anyhow::bail!("rate limits must allow at least one request in a burst");
        }
        Ok(())
    }

    /// The filesystem path of the SQLite database, when one is in use.
    #[must_use]
    pub fn sqlite_file(&self) -> Option<PathBuf> {
        if !self.database.url.starts_with("sqlite") {
            return None;
        }
        let rest = self
            .database
            .url
            .trim_start_matches("sqlite://")
            .trim_start_matches("sqlite:");
        let path = rest.split('?').next().unwrap_or_default();
        (!path.is_empty() && path != ":memory:").then(|| PathBuf::from(path))
    }

    /// Whether the configuration came from a file on disk.
    #[must_use]
    pub fn has_config_file(&self) -> bool {
        self.config_path.is_some()
    }
}

// ---------------------------------------------------------------------------
// File representation
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    environment: Option<String>,
    site: Option<SiteSection>,
    server: Option<ServerSection>,
    database: Option<DatabaseSection>,
    storage: Option<StorageSection>,
    security: Option<SecuritySection>,
    logging: Option<LoggingSection>,
    assets: Option<AssetsSection>,
    dev: Option<DevSection>,
    accounts: Option<AccountsSection>,
    age: Option<AgeSection>,
    rate_limits: Option<RateLimitSection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteSection {
    name: Option<String>,
    base_url: Option<String>,
    contact_email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerSection {
    bind: Option<String>,
    port: Option<u16>,
    max_body_bytes: Option<usize>,
    request_timeout_secs: Option<u64>,
    cors_origins: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseSection {
    url: Option<String>,
    max_connections: Option<u32>,
    acquire_timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorageSection {
    root: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecuritySection {
    cookie_secure: Option<bool>,
    session_ttl_days: Option<u32>,
    csrf_required: Option<bool>,
    trust_proxy: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountsSection {
    registration_open: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgeSection {
    threshold: Option<u8>,
    guardian_workflow_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RateLimitSection {
    auth_burst: Option<u32>,
    auth_per_minute: Option<u32>,
    write_burst: Option<u32>,
    write_per_minute: Option<u32>,
    search_burst: Option<u32>,
    search_per_minute: Option<u32>,
    default_burst: Option<u32>,
    default_per_minute: Option<u32>,
    address_multiplier: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoggingSection {
    filter: Option<String>,
    format: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetsSection {
    dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevSection {
    seed_enabled: Option<bool>,
}

/// Ensure a directory exists, creating it when necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("creating directory {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_development_friendly() {
        let config = Config::load(&GlobalArgs::default()).expect("loads");
        assert_eq!(config.environment, Environment::Development);
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.bind, "127.0.0.1");
        assert!(!config.security.cookie_secure);
        assert!(config.dev.seed_enabled);
        assert!(config.assets.dir.is_none());
        assert!(config.database.url.starts_with("sqlite://"));
    }

    #[test]
    fn environment_variables_beat_defaults() {
        let args = GlobalArgs {
            port: Some(4321),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.server.port, 4321);
        // The default base_url follows the resolved port, so links stay correct.
        assert_eq!(config.site.base_url, "http://localhost:4321");
    }

    #[test]
    fn production_flips_the_safe_defaults() {
        let args = GlobalArgs {
            environment: Some("production".to_owned()),
            storage_root: Some(PathBuf::from("/tmp/lorehaven-test")),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert!(config.environment.is_production());
        assert!(
            config.security.cookie_secure,
            "cookies must be Secure in production"
        );
        assert!(
            !config.dev.seed_enabled,
            "seeding must be off in production"
        );
        assert_eq!(config.logging.format, LogFormat::Json);
    }

    #[test]
    fn unknown_environments_are_refused() {
        let args = GlobalArgs {
            environment: Some("staging".to_owned()),
            ..GlobalArgs::default()
        };
        assert!(Config::load(&args).is_err());
    }

    #[test]
    fn invalid_values_are_refused_with_a_clear_rule() {
        let mut config = Config::development_defaults();
        config.server.port = 0;
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.server.max_body_bytes = 10;
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.site.base_url = "ftp://example.com".to_owned();
        assert!(config.validate().is_err());

        let mut config = Config::development_defaults();
        config.site.name = "  ".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_config_file_is_read_and_overridden_by_the_environment() {
        let dir = std::env::temp_dir().join(format!("lorehaven-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(
            &path,
            r#"
environment = "development"

[site]
name = "Test Haven"

[server]
port = 7000
"#,
        )
        .expect("write config");

        let args = GlobalArgs {
            config: Some(path.clone()),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.site.name, "Test Haven");
        assert_eq!(config.server.port, 7000);

        // An explicit argument still wins over the file.
        let args = GlobalArgs {
            config: Some(path.clone()),
            port: Some(7001),
            ..GlobalArgs::default()
        };
        let config = Config::load(&args).expect("loads");
        assert_eq!(config.server.port, 7001);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_config_keys_are_refused() {
        let dir = std::env::temp_dir().join(format!("lorehaven-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, "[server]\nprot = 7000\n").expect("write config");

        let args = GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        };
        let error = Config::load(&args).expect_err("must refuse the typo");
        assert!(
            format!("{error:#}").contains("prot"),
            "the error should name the offending key: {error:#}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_explicit_config_file_is_an_error() {
        let args = GlobalArgs {
            config: Some(PathBuf::from("/nonexistent/lorehaven.toml")),
            ..GlobalArgs::default()
        };
        assert!(Config::load(&args).is_err());
    }

    #[test]
    fn the_sqlite_path_is_extracted_for_backup_and_doctor_checks() {
        let mut config = Config::development_defaults();
        config.database.url = "sqlite://./data/lorehaven.sqlite?mode=rwc".to_owned();
        assert_eq!(
            config.sqlite_file().expect("path"),
            PathBuf::from("./data/lorehaven.sqlite")
        );

        config.database.url = "postgres://user@host/lorehaven".to_owned();
        assert!(config.sqlite_file().is_none());

        config.database.url = "sqlite://:memory:".to_owned();
        assert!(config.sqlite_file().is_none());
    }
}
