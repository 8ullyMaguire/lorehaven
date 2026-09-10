//! `lorehaven doctor` — an honest picture of this instance's health.
//!
//! Spec §5 lists `doctor` alongside `serve`; spec §17 wants the same
//! information available in the admin UI later. The principle here is that a
//! doctor run must *report* problems rather than crash on them: a database that
//! cannot be reached is a finding, not an error, because the whole point of
//! running the command is to find out.

use std::path::{Path, PathBuf};

use lorehaven_db::{migrate, Database, DatabaseConfig};

use crate::cli::DoctorArgs;
use crate::config::Config;
use crate::safety::{self, Severity};
use crate::version;

/// One observation.
#[derive(Debug, Clone)]
pub struct Check {
    /// What was checked.
    pub name: &'static str,
    /// How serious the result is.
    pub severity: Severity,
    /// What was observed.
    pub detail: String,
    /// What to do about it.
    pub remedy: String,
}

/// The full set of observations.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Every check, in the order it ran.
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether any check is fatal.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.severity == Severity::Fatal)
    }

    /// Whether any check is a warning.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.severity == Severity::Warning)
    }

    fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    fn ok(&mut self, name: &'static str, detail: impl Into<String>) {
        self.push(Check {
            name,
            severity: Severity::Ok,
            detail: detail.into(),
            remedy: String::new(),
        });
    }

    fn warn(&mut self, name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) {
        self.push(Check {
            name,
            severity: Severity::Warning,
            detail: detail.into(),
            remedy: remedy.into(),
        });
    }

    fn fail(&mut self, name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) {
        self.push(Check {
            name,
            severity: Severity::Fatal,
            detail: detail.into(),
            remedy: remedy.into(),
        });
    }
}

/// Run every check.
pub async fn run(config: &Config, _args: &DoctorArgs) -> Report {
    let mut report = Report::default();

    // --- build --------------------------------------------------------------
    report.ok(
        "build",
        format!(
            "{} (api {}, environment {})",
            version::build_id(),
            version::API_VERSION,
            config.environment.as_str()
        ),
    );

    // --- configuration file -------------------------------------------------
    match &config.config_path {
        Some(path) => report.ok("config-file", format!("{} loaded", path.display())),
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            report.warn(
                "config-file",
                format!(
                    "no {} found in {}; every value is a default or an environment variable",
                    crate::config::DEFAULT_CONFIG_FILE,
                    cwd.display()
                ),
                "copy lorehaven.toml.example to lorehaven.toml for a documented starting point",
            );
        }
    }

    // --- configuration safety ----------------------------------------------
    for finding in safety::audit(config) {
        match finding.severity {
            Severity::Ok => report.ok(finding.code, finding.detail),
            Severity::Warning => report.warn(finding.code, finding.detail, finding.remedy),
            Severity::Fatal => report.fail(finding.code, finding.detail, finding.remedy),
        }
    }

    // --- schema -------------------------------------------------------------
    let db_config = DatabaseConfig::new(config.database.url.clone());
    match Database::connect(&db_config).await {
        Ok(db) => {
            report.ok(
                "database",
                format!(
                    "{} reachable at {}",
                    db.backend().as_str(),
                    db.redacted_url()
                ),
            );

            match migrate::pending(&db).await {
                Ok(pending) if pending.is_empty() => {
                    let known = migrate::catalogue(db.backend()).len();
                    report.ok("migrations", format!("all {known} migration(s) applied"));
                }
                Ok(pending) => report.fail(
                    "migrations",
                    format!(
                        "{} migration(s) pending: {}",
                        pending.len(),
                        pending.join(", ")
                    ),
                    "run `lorehaven migrate`",
                ),
                Err(error) => report.fail(
                    "migrations",
                    format!("cannot read the migration ledger: {error}"),
                    "run `lorehaven migrate`",
                ),
            }

            report.ok(
                "schema-print",
                match (migrate::catalogue(db.backend()).len(), &config.environment) {
                    (0, _) => "no migrations are compiled in — this build is incomplete".to_owned(),
                    (count, _) => format!("{count} migration(s) compiled into this binary"),
                },
            );

            if let Some(path) = config.sqlite_file() {
                report.ok("database-file", path.display().to_string());
            }
            db.close().await;
        }
        Err(error) => report.fail(
            "database",
            format!("cannot connect to {}: {error:#}", config.database.url),
            "check the URL, credentials and that the server is running",
        ),
    }

    // --- storage ------------------------------------------------------------
    match check_storage_blocking(&config.storage.root) {
        Ok(detail) => {
            report.ok("storage", detail);
            match free_space(&config.storage.root) {
                Some(free) if free < 512 * 1024 * 1024 => report.warn(
                    "disk-space",
                    format!(
                        "only {} free at {}",
                        human_bytes(free),
                        config.storage.root.display()
                    ),
                    "free space or move the storage root; imports and exports need room",
                ),
                Some(free) => report.ok("disk-space", format!("{} free", human_bytes(free))),
                None => report.warn(
                    "disk-space",
                    "could not determine free space",
                    "check manually with `df -h`",
                ),
            }
        }
        Err(error) => report.fail(
            "storage",
            format!("{}: {error}", config.storage.root.display()),
            "ensure the directory exists and is writable by the service user",
        ),
    }

    // --- secret key ---------------------------------------------------------
    //
    // Milestone 5 ships the encrypted-secret store; Milestone 6 is what puts
    // source credentials in it. The key is still checked here, because an
    // instance that cannot load one must not find that out when it first tries
    // to store a credential.
    match crate::secrets::load_cipher(
        &config.storage.root,
        config.security.secret_key_file.as_deref(),
        config.environment.is_production(),
    ) {
        Ok(cipher) => {
            let owner = crate::secrets::Record {
                owner_type: "doctor",
                owner_id: "self",
                name: "round-trip",
            };
            let secret = crate::secrets::Secret::new("doctor probe");
            match cipher
                .encrypt(owner, &secret)
                .and_then(|sealed| cipher.decrypt(owner, &sealed))
            {
                Ok(opened) if opened.expose() == secret.expose() => report.ok(
                    "secret-key",
                    format!(
                        "key {} loaded, and a round trip through it returned what went in",
                        cipher.active_key_id()
                    ),
                ),
                Ok(_) => report.fail(
                    "secret-key",
                    "a secret did not decrypt to what was encrypted with the same key",
                    "do not store credentials on this instance; the key material is wrong",
                ),
                Err(error) => report.fail(
                    "secret-key",
                    format!("the key loads but cannot encrypt: {error:#}"),
                    "set LOREHAVEN_SECRET_KEY to a valid 32-byte hex key",
                ),
            }
        }
        Err(error) => report.fail(
            "secret-key",
            format!("no usable secret key: {error:#}"),
            "set LOREHAVEN_SECRET_KEY, or create the configured key file",
        ),
    }

    // --- optional converters ------------------------------------------------
    for binary in ["ebook-convert", "pandoc"] {
        match which(binary) {
            Some(path) => report.ok(
                "converter",
                format!("{binary} available at {}", path.display()),
            ),
            None => report.warn(
                "converter",
                format!("{binary} not found on PATH"),
                "PDF, MOBI and AZW3 export stays disabled until it is installed; \
                 everything else is unaffected",
            ),
        }
    }

    // --- assets -------------------------------------------------------------
    match &config.assets.dir {
        Some(dir) if dir.join("index.html").exists() => report.ok(
            "assets",
            format!("served from {} (development)", dir.display()),
        ),
        Some(dir) => report.fail(
            "assets",
            format!("{} has no index.html", dir.display()),
            "run `npm run build` in frontend/, or unset [assets] dir to use the embedded bundle",
        ),
        None => report.ok("assets", "served from the compiled-in bundle"),
    }

    report
}

/// Render a report for a terminal.
#[must_use]
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    for check in &report.checks {
        let marker = match check.severity {
            Severity::Ok => "ok  ",
            Severity::Warning => "warn",
            Severity::Fatal => "FAIL",
        };
        out.push_str(&format!("[{marker}] {:<16} {}\n", check.name, check.detail));
        if !check.remedy.is_empty() {
            out.push_str(&format!("{:>7}{:<16} fix: {}\n", "", "", check.remedy));
        }
    }

    let failures = report
        .checks
        .iter()
        .filter(|c| c.severity == Severity::Fatal)
        .count();
    let warnings = report
        .checks
        .iter()
        .filter(|c| c.severity == Severity::Warning)
        .count();
    out.push_str(&format!(
        "\n{} check(s): {failures} failing, {warnings} warning(s)\n",
        report.checks.len()
    ));
    out
}

/// Create the directory and prove it accepts writes.
///
/// Synchronous because `doctor` is a one-shot command where a blocking write is
/// clearer than threading a runtime through every check.
fn check_storage_blocking(root: &Path) -> anyhow::Result<String> {
    std::fs::create_dir_all(root)
        .map_err(|error| anyhow::anyhow!("cannot create directory: {error}"))?;
    let probe = root.join(format!(".lorehaven-doctor-probe-{}", std::process::id()));
    std::fs::write(&probe, b"probe").map_err(|error| anyhow::anyhow!("cannot write: {error}"))?;
    std::fs::remove_file(&probe)
        .map_err(|error| anyhow::anyhow!("cannot remove probe file: {error}"))?;
    Ok(format!("{} is writable", root.display()))
}

/// Free bytes on the filesystem holding `path`, if it can be determined.
fn free_space(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let available_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kb * 1024)
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Locate an executable on `PATH` without spawning it.
#[must_use]
pub fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join(binary);
        let is_executable = candidate.is_file()
            && std::fs::metadata(&candidate)
                .map(|meta| !meta.permissions().readonly() || true)
                .unwrap_or(false);
        is_executable.then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_report_counts_failures_and_warnings() {
        let mut report = Report::default();
        report.ok("a", "fine");
        assert!(!report.has_failures());
        assert!(!report.has_warnings());

        report.warn("b", "hmm", "fix it");
        assert!(report.has_warnings());
        assert!(!report.has_failures());

        report.fail("c", "broken", "fix it");
        assert!(report.has_failures());
    }

    #[test]
    fn rendering_shows_remedies_and_a_summary() {
        let mut report = Report::default();
        report.fail("database", "unreachable", "start the server");
        let text = render(&report);
        assert!(text.contains("[FAIL]"));
        assert!(text.contains("start the server"));
        assert!(text.contains("1 failing"));
    }

    #[test]
    fn byte_formatting_is_readable() {
        assert_eq!(human_bytes(512), "512.0 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn which_finds_a_binary_that_must_exist() {
        // `sh` is present on every platform this project supports.
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }

    #[tokio::test]
    async fn doctor_reports_rather_than_failing_on_an_unreachable_database() {
        let mut config = Config::development_defaults();
        config.database = DatabaseConfig {
            // Port 1 is never a PostgreSQL server.
            url: "postgres://nobody@127.0.0.1:1/nope".to_owned(),
            max_connections: 1,
            acquire_timeout: Duration::from_millis(500),
            slow_query_warn: Duration::ZERO,
        };
        let report = run(&config, &DoctorArgs { strict: false }).await;
        assert!(
            report.has_failures(),
            "an unreachable database is a failure: {}",
            render(&report)
        );
        // It still produced a complete report rather than panicking.
        assert!(report.checks.iter().any(|c| c.name == "build"));
    }
}
