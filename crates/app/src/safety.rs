//! Startup safety checks.
//!
//! Spec §5, acceptance: "Production startup rejects unsafe development
//! configuration."
//!
//! The rule this module encodes: an insecure default is acceptable in
//! development and unacceptable in production, and the difference must be
//! *enforced*, not documented. Each finding carries a severity so that `doctor`
//! can show a full picture while `serve` refuses to boot on anything fatal.

use crate::config::Config;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Fine.
    Ok,
    /// Works, but the operator should know.
    Warning,
    /// Must not start.
    Fatal,
}

/// A single observation about the configuration.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Stable identifier, useful in tests and support conversations.
    pub code: &'static str,
    /// How serious it is.
    pub severity: Severity,
    /// What was observed.
    pub detail: String,
    /// What to do about it.
    pub remedy: String,
}

impl Finding {
    fn ok(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Ok,
            detail: detail.into(),
            remedy: String::new(),
        }
    }

    fn warning(code: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            detail: detail.into(),
            remedy: remedy.into(),
        }
    }

    fn fatal(code: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Fatal,
            detail: detail.into(),
            remedy: remedy.into(),
        }
    }
}

/// Audit the configuration for the active environment.
#[must_use]
pub fn audit(config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let production = config.environment.is_production();

    // --- cookies ------------------------------------------------------------
    if config.security.cookie_secure {
        findings.push(Finding::ok(
            "cookie-secure",
            "session cookies carry the Secure attribute",
        ));
    } else if production {
        findings.push(Finding::fatal(
            "cookie-secure",
            "session cookies would be sent over plain HTTP in production",
            "set [security] cookie_secure = true (and serve the site over HTTPS)",
        ));
    } else {
        findings.push(Finding::ok(
            "cookie-secure",
            "cookies are not marked Secure; expected outside production",
        ));
    }

    // --- CSRF ---------------------------------------------------------------
    if config.security.csrf_required {
        findings.push(Finding::ok(
            "csrf",
            "state-changing cookie-authenticated requests require a CSRF token",
        ));
    } else if production {
        findings.push(Finding::fatal(
            "csrf",
            "CSRF protection is disabled in production",
            "set [security] csrf_required = true",
        ));
    } else {
        findings.push(Finding::warning(
            "csrf",
            "CSRF protection is disabled",
            "only acceptable in development or tests",
        ));
    }

    // --- dev affordances ----------------------------------------------------
    if config.dev.seed_enabled && production {
        findings.push(Finding::fatal(
            "dev-seeding",
            "the development seeder is enabled in production",
            "set [dev] seed_enabled = false",
        ));
    }

    // --- assets -------------------------------------------------------------
    match &config.assets.dir {
        Some(dir) if production => findings.push(Finding::fatal(
            "assets-from-disk",
            format!(
                "assets would be served from {} in production",
                dir.display()
            ),
            "remove [assets] dir so the compiled-in bundle is served",
        )),
        Some(dir) => findings.push(Finding::warning(
            "assets-from-disk",
            format!(
                "assets are served from {} instead of the embedded bundle",
                dir.display()
            ),
            "expected in development; unset before shipping",
        )),
        None => findings.push(Finding::ok(
            "assets-from-disk",
            "assets are served from the compiled-in bundle",
        )),
    }

    // --- public URL ---------------------------------------------------------
    if production && !config.site.base_url.starts_with("https://") {
        findings.push(Finding::fatal(
            "base-url-scheme",
            format!(
                "site.base_url is {:?}, which is not HTTPS",
                config.site.base_url
            ),
            "set site.base_url to the public HTTPS address",
        ));
    } else {
        findings.push(Finding::ok(
            "base-url-scheme",
            format!("site.base_url is {}", config.site.base_url),
        ));
    }

    // --- bind address -------------------------------------------------------
    let loopback = is_loopback(&config.server.bind);
    if production && !loopback {
        findings.push(Finding::warning(
            "bind-address",
            format!(
                "listening on {} directly rather than a loopback address",
                config.server.bind
            ),
            "bind to 127.0.0.1 and terminate TLS in a reverse proxy, unless \
             the host itself is the only network boundary",
        ));
    } else {
        findings.push(Finding::ok(
            "bind-address",
            format!("listening on {}", config.server.bind),
        ));
    }

    // --- CORS ---------------------------------------------------------------
    if config
        .server
        .cors_origins
        .iter()
        .any(|origin| origin == "*")
    {
        findings.push(if production {
            Finding::fatal(
                "cors-wildcard",
                "CORS allows every origin in production",
                "list the deployed origins explicitly",
            )
        } else {
            Finding::warning(
                "cors-wildcard",
                "CORS allows every origin",
                "list the deployed origins explicitly before shipping",
            )
        });
    } else {
        findings.push(Finding::ok(
            "cors",
            if config.server.cors_origins.is_empty() {
                "same-origin only".to_owned()
            } else {
                format!("{} allowed origin(s)", config.server.cors_origins.len())
            },
        ));
    }

    // --- data location ------------------------------------------------------
    let default_data_dir = config.storage.root == std::path::Path::new("./data");
    if production && default_data_dir {
        findings.push(Finding::fatal(
            "storage-root",
            "the storage root is still the relative development path ./data",
            "set [storage] root to an absolute path such as /var/lib/lorehaven",
        ));
    }

    // --- logging ------------------------------------------------------------
    if production && config.logging.format == crate::config::LogFormat::Pretty {
        findings.push(Finding::warning(
            "log-format",
            "production logs are human-readable rather than JSON",
            "set [logging] format = \"json\" if a collector parses them",
        ));
    }

    findings
}

/// Refuse to start when any fatal finding is present.
pub fn validate_for_startup(config: &Config) -> anyhow::Result<()> {
    let fatal: Vec<Finding> = audit(config)
        .into_iter()
        .filter(|finding| finding.severity == Severity::Fatal)
        .collect();

    if fatal.is_empty() {
        return Ok(());
    }

    let mut message = format!(
        "refusing to start in the {} environment:\n",
        config.environment.as_str()
    );
    for finding in &fatal {
        message.push_str(&format!(
            "  - [{}] {}\n      fix: {}\n",
            finding.code, finding.detail, finding.remedy
        ));
    }
    anyhow::bail!("{message}")
}

/// Whether an address string is a loopback address.
fn is_loopback(bind: &str) -> bool {
    matches!(bind, "localhost" | "::1" | "[::1]")
        || bind
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Environment;

    #[test]
    fn development_defaults_have_no_fatal_findings() {
        let config = Config::development_defaults();
        validate_for_startup(&config).expect("development must start");
    }

    #[test]
    fn production_refuses_insecure_defaults() {
        let mut config = Config::development_defaults();
        config.environment = Environment::Production;
        // Deliberately leave the development values in place.
        let error = validate_for_startup(&config).expect_err("must refuse");
        let message = format!("{error}");
        assert!(message.contains("cookie-secure"), "{message}");
        assert!(message.contains("dev-seeding"), "{message}");
        assert!(message.contains("base-url-scheme"), "{message}");
        assert!(message.contains("storage-root"), "{message}");
    }

    #[test]
    fn a_properly_configured_production_starts() {
        let mut config = Config::development_defaults();
        config.environment = Environment::Production;
        config.security.cookie_secure = true;
        config.dev.seed_enabled = false;
        config.assets.dir = None;
        config.storage.root = "/var/lib/lorehaven".into();
        config.site.base_url = "https://lorehaven.example".to_owned();
        config.logging.format = crate::config::LogFormat::Json;
        config.server.bind = "127.0.0.1".to_owned();
        validate_for_startup(&config).expect("production must start");
    }

    #[test]
    fn a_wildcard_cors_origin_is_fatal_in_production() {
        let mut config = Config::development_defaults();
        config.environment = Environment::Production;
        config.security.cookie_secure = true;
        config.dev.seed_enabled = false;
        config.storage.root = "/var/lib/lorehaven".into();
        config.site.base_url = "https://lorehaven.example".to_owned();
        config.server.cors_origins = vec!["*".to_owned()];
        let error = validate_for_startup(&config).expect_err("must refuse");
        assert!(format!("{error}").contains("cors-wildcard"));
    }

    #[test]
    fn loopback_detection_covers_the_usual_spellings() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.1.1"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("10.0.0.5"));
    }

    #[test]
    fn plain_http_in_production_is_fatal() {
        let mut config = Config::development_defaults();
        config.environment = Environment::Production;
        config.security.cookie_secure = true;
        config.dev.seed_enabled = false;
        config.storage.root = "/var/lib/lorehaven".into();
        config.site.base_url = "http://lorehaven.example".to_owned();
        let error = validate_for_startup(&config).expect_err("must refuse");
        assert!(format!("{error}").contains("base-url-scheme"));
    }
}
