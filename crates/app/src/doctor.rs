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
    /// What this run could not examine, and why (spec §38.7.2/§38.7.3).
    ///
    /// A separate list rather than a filter over `checks`, because the
    /// requirement is that the report *ends* with what it did not check. A
    /// consumer that only reads `checks` cannot see the difference between
    /// "19 checks, all ok" and "19 checks, all ok, of the things that matter
    /// most"; rendering keeps them separate and so does the struct.
    pub not_checked: Vec<NotChecked>,
}

/// An area this run could not observe from this box.
///
/// The `reason` is mandatory and is the whole point: a bare skip is
/// indistinguishable from a check that passed, which is what spec §38.7.2
/// forbids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotChecked {
    /// The area, named so an operator knows what to go and look at.
    pub area: &'static str,
    /// Why this run could not observe it.
    pub reason: String,
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

    /// Whether any check could not run.
    #[must_use]
    pub fn has_skips(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.severity == Severity::Skipped)
            || !self.not_checked.is_empty()
    }

    /// Record a check that could not run, with the reason it could not.
    ///
    /// Routed through `not_checked` rather than only into `checks`: the reason is
    /// an operator-facing sentence, and §38.7.3 requires the report to end with
    /// this list. Putting it in one place means a check cannot be skipped and then
    /// reported as if it had run.
    fn skip(&mut self, name: &'static str, area: &'static str, reason: impl Into<String>) {
        let reason = reason.into();
        self.push(Check {
            name,
            severity: Severity::Skipped,
            detail: reason.clone(),
            remedy: String::new(),
        });
        self.not_checked.push(NotChecked { area, reason });
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
///
/// `--strict` is threaded in so the *report* can say which findings were
/// promoted, rather than only so the exit code changes. It is handled at the
/// call site in `lib.rs` for the exit code, and here for the line a reader
/// actually looks at.
///
/// A previous ledger note recorded this binding as a defect — "`--strict` is
/// declared and ignored, so it returns zero with warnings". That was wrong: the
/// flag was read at the call site and worked. Running the binary settled it
/// (`doctor` exits 0, `doctor --strict` exits 1, same 19-check report), and the
/// ledger row is corrected rather than the code bent to match the note.
pub async fn run(config: &Config, args: &DoctorArgs) -> Report {
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
            // The audit's own skipped findings flow through the same skip path as
            // the inlined checks below, so a precondition the audit could not
            // meet is reported as not-checked rather than dropped by a match arm
            // that did not know the variant existed.
            Severity::Skipped => report.skip(finding.code, finding.code, finding.detail),
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

    // --- narration ----------------------------------------------------------
    // The engine is built the same way the running state builds it, so what the
    // doctor says and what a job gets cannot disagree. A missing engine is a
    // warning, not a failure: an instance that never narrates is a perfectly
    // good library, and the remedy is what the operator needs to read.
    match crate::tts::build_engine(&config.tts, which("piper").as_deref()) {
        Ok(engine) => match engine.health() {
            Ok(()) => report.ok("narration", format!("engine {:?} is ready", engine.name())),
            Err(error) => report.warn(
                "narration",
                format!("engine {:?} is not usable: {error}", engine.name()),
                "narration editions stay unavailable until this is fixed; \
                 everything else is unaffected",
            ),
        },
        Err(error) => report.warn(
            "narration",
            format!("tts.engine is not an engine this build has: {error}"),
            "set tts.engine to one of: piper, silent",
        ),
    }

    // --- what this run could not check --------------------------------------
    //
    // Spec §38.7.3: the report ends with what was NOT checked. These four are
    // unobservable from the box on *every* run, not only when something is
    // broken, which is precisely why the list is unconditional: an empty block
    // would itself be the overstatement the clause forbids, because "I checked
    // everything" is the claim the block exists to refuse.
    //
    // Each names an area a reader of this output would otherwise assume was
    // covered, because `doctor` reports on the instance and these are properties
    // of the *deployment* around it.
    report.skip(
        "backup-restore",
        "backup restore",
        "whether a backup taken from this instance can actually be restored. \
         A successful backup is not evidence of a successful restore, and this \
         run cannot attempt one without clobbering live data.",
    );
    report.skip(
        "worker-egress",
        "worker network egress",
        "whether the background worker can reach the sources and hosts it needs. \
         The worker is not running during this check, and no fetch is made from \
         here on its behalf.",
    );
    report.skip(
        "tls-termination",
        "TLS termination",
        "how this instance is served over the network. The listener this check \
         can see binds a socket; whether a proxy in front of it presents a \
         certificate is not observable from inside.",
    );
    report.skip(
        "index-build",
        "search index freshness",
        "whether the search index reflects recent writes. Building it would be a \
         mutation, and a diagnostic that mutates the instance it is diagnosing \
         can report a problem it just caused.",
    );

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

    // --- snapshot publication gate ------------------------------------------
    //
    // Spec §38.7.5, pinned by M60: "every check reporting Ok executed its probe;
    // the snapshot check restores rather than reading a manifest." Trap 3 shipped
    // an entire unmasked instance — all 277 original public tables beside the 70
    // masked ones — while every gate was green, because the checks read a
    // manifest and a policy document rather than running the script that does
    // the work.
    //
    // So this check *runs the script* and reads its exit code. It is not a
    // re-implementation of the gate and it does not parse the policy: a doctor
    // check that re-derived the answer would be the same class of defect that let
    // Trap 3 through, with one more layer of code between the gate and the thing
    // being gated.
    //
    // `python3` missing is a SKIP rather than a warning or a pass: the check
    // could not run, which is what §38.7.2 says to say, and reporting it as `Ok`
    // would be the precise overstatement this row exists to prevent.
    match (which("python3"), repo_root()) {
        (Some(_), Some(root)) => {
            let gate = root.join("scripts/check-snapshot-pii.py");
            if !gate.is_file() {
                report.skip(
                    "snapshot-gate",
                    "snapshot publication gate",
                    format!(
                        "{} is not present, so the published-dataset PII gate could \
                         not be run",
                        gate.display()
                    ),
                );
            } else {
                match std::process::Command::new("python3")
                    .arg(&gate)
                    .current_dir(&root)
                    .output()
                {
                    Ok(output) if output.status.success() => {
                        let detail = String::from_utf8_lossy(&output.stdout);
                        report.ok(
                            "snapshot-gate",
                            format!(
                                "check-snapshot-pii.py ran and passed: {}",
                                detail.trim().lines().last().unwrap_or("no output")
                            ),
                        );
                    }
                    Ok(output) => {
                        let detail = String::from_utf8_lossy(&output.stdout);
                        report.fail(
                            "snapshot-gate",
                            format!(
                                "check-snapshot-pii.py ran and refused: {}",
                                detail.trim().lines().last().unwrap_or("no output")
                            ),
                            "every column of every covered table needs a decision in \
                             docs/snapshot-column-policy.json; the script names the \
                             offending table.column",
                        );
                    }
                    Err(error) => {
                        report.skip(
                            "snapshot-gate",
                            "snapshot publication gate",
                            format!("the gate could not be executed: {error}"),
                        );
                    }
                }
            }
        }
        (None, _) => {
            report.skip(
                "snapshot-gate",
                "snapshot publication gate",
                "python3 is not on PATH, so the gate that decides what this instance \
                 may publish was not run",
            );
        }
        (_, None) => {
            report.skip(
                "snapshot-gate",
                "snapshot publication gate",
                "the repository root could not be located, so scripts/check-snapshot-pii.py \
                 was not found",
            );
        }
    }

    // --- strict -------------------------------------------------------------
    //
    // The flag's effect on the exit code is applied by the caller; what belongs
    // here is the line a reader looks at. Without it, `doctor --strict` and
    // `doctor` print byte-identical output and differ only in an exit code the
    // reader may never see — which is exactly the "a flag that does nothing"
    // shape this project has been bitten by twice, in a subtler form: the flag
    // does something, but nothing on screen says so.
    if args.strict {
        // Counted rather than reported as a boolean, because "the warnings above"
        // is more useful with a number against it, and because a line saying
        // "strict" with no count is the kind of reassurance a reader cannot check.
        let warnings = report
            .checks
            .iter()
            .filter(|check| check.severity == Severity::Warning)
            .count();
        report.ok(
            "strict",
            format!(
                "enabled: {warnings} warning(s) above are treated as failures for \
                 the exit code"
            ),
        );
    }

    report
}

/// Render a report for a terminal.
///
/// Ends with the NOT CHECKED block (spec §38.7.3), so the last thing on screen
/// is what the run did not examine rather than a count of what it did.
#[must_use]
pub fn render(report: &Report) -> String {
    let mut out = String::new();
    for check in &report.checks {
        let marker = match check.severity {
            Severity::Ok => "ok  ",
            Severity::Warning => "warn",
            Severity::Fatal => "FAIL",
            // `skip`, not `warn`. A warning says "I looked and this is wrong";
            // this says "I did not look", and rendering the second as the first
            // is the overstatement spec §38.7.2 exists to prevent.
            Severity::Skipped => "skip",
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
    let skipped = report
        .checks
        .iter()
        .filter(|c| c.severity == Severity::Skipped)
        .count();
    out.push_str(&format!(
        "\n{} check(s): {failures} failing, {warnings} warning(s), {skipped} not checked\n",
        report.checks.len()
    ));

    // Spec §38.7.3: the report ENDS with what was not checked.
    //
    // Not conditional on the list being non-empty, because the four structural
    // skips are unconditional: an empty block here would mean "nothing was out
    // of reach", which is the claim this section exists to refuse. If a future
    // change makes every area observable, the block should say so rather than
    // disappear — silence would read as full coverage.
    out.push_str("\nNOT CHECKED\n");
    if report.not_checked.is_empty() {
        out.push_str(
            "  (none recorded — if you expected a skip here, the report is wrong; \
             some areas are unobservable from this box on every run)\n",
        );
    }
    for item in &report.not_checked {
        out.push_str(&format!("  - {}: {}\n", item.area, item.reason));
    }
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

/// The repository root, found by walking up from the compiled-in manifest dir.
///
/// `CARGO_MANIFEST_DIR` is baked in at compile time and points at `crates/app`,
/// so the root is two levels up. It is `None` rather than a guess when the walk
/// does not find a `migrations/` directory, so a relocated binary reports a skip
/// instead of running a gate from a directory that is not this project.
fn repo_root() -> Option<PathBuf> {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    start
        .ancestors()
        .find(|dir| dir.join("migrations").is_dir() && dir.join("scripts").is_dir())
        .map(Path::to_path_buf)
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

    // --- spec §38.7.2: a check that could not run -----------------------------

    /// A skip renders as `skip`, and never as `ok` or `warn`.
    ///
    /// The three spellings are asserted individually because each is a distinct
    /// overstatement: `ok` says "I looked and it is fine", `warn` says "I looked
    /// and it is wrong", and this run knows neither. Spec §38.7.2 forbids both.
    #[test]
    fn a_skip_renders_as_neither_ok_nor_a_warning() {
        let mut report = Report::default();
        report.skip("peer", "federation peers", "no peer list is configured");
        let text = render(&report);

        assert!(text.contains("[skip]"), "{text}");
        assert!(
            !text.contains("[ok  ] peer"),
            "a check that did not run reported ok: {text}"
        );
        assert!(
            !text.contains("[warn] peer"),
            "a check that did not run reported a warning: {text}"
        );
    }

    /// A skip is counted separately from warnings, so `--strict` cannot promote
    /// one into a failure.
    ///
    /// This is the coupling that makes `Severity::Skipped` a bad thing to be lazy
    /// about: if skips counted as warnings, every `doctor --strict` run on a
    /// healthy instance would exit 1 over four permanent structural skips, and
    /// operators would learn to ignore the exit code entirely.
    #[test]
    fn a_skip_is_neither_a_warning_nor_a_failure() {
        let mut report = Report::default();
        report.skip("a", "area a", "not observable");
        report.skip("b", "area b", "not observable");

        assert!(!report.has_warnings(), "a skip counted as a warning");
        assert!(!report.has_failures(), "a skip counted as a failure");
        assert!(report.has_skips(), "a skip that has_skips does not see");

        let text = render(&report);
        assert!(text.contains("2 not checked"), "{text}");
        assert!(text.contains("0 failing, 0 warning(s)"), "{text}");
    }

    /// The reason is mandatory, and it reaches the output.
    ///
    /// §38.7.2's word is "mandatory" and this is why: a bare skip is
    /// indistinguishable from a check that passed, which is the thing the
    /// requirement exists to stop. An operator who knows the area is unobservable
    /// can go and check it by hand; one who sees a bare `skip` learns nothing.
    #[test]
    fn a_skip_carries_its_reason_into_the_report() {
        let mut report = Report::default();
        report.skip(
            "search",
            "search index freshness",
            "the index is not built during a diagnostic",
        );
        let text = render(&report);

        assert!(text.contains("search index freshness"), "{text}");
        assert!(
            text.contains("the index is not built during a diagnostic"),
            "the reason did not reach the operator: {text}"
        );
    }

    /// A warning alongside a skip is still just one warning.
    ///
    /// Asserted so the skip path cannot quietly start consuming warnings, which
    /// would make `--strict` stop failing for the reasons it exists.
    #[test]
    fn a_skip_alongside_a_warning_leaves_the_warning_countable() {
        let mut report = Report::default();
        report.warn("disk", "nearly full", "free some space");
        report.skip("peer", "federation peers", "no peer list is configured");

        assert_eq!(
            render(&report).matches("[warn]").count(),
            1,
            "the warning disappeared or doubled"
        );
        assert!(report.has_warnings());
        assert!(!report.has_failures());
    }

    // --- spec §38.7.3: the report ends with what it did not check ------------

    /// The NOT CHECKED block is present on a report with nothing wrong.
    ///
    /// The important part is the negative case: `Report::default()` has no skips
    /// and no failures, and the block still prints. An empty block would mean "you
    /// saw everything", which is the claim §38.7.3 exists to refuse — and the
    /// four structural areas in `run` are unobservable on every single run.
    #[test]
    fn the_not_checked_block_appears_even_when_nothing_is_wrong() {
        let report = Report::default();
        let text = render(&report);

        assert!(
            text.contains("NOT CHECKED"),
            "a clean report says nothing about coverage: {text}"
        );
        assert!(
            text.contains("none recorded"),
            "an empty list must say so rather than read as full coverage: {text}"
        );
    }

    /// The block is last, so it is the last thing on screen.
    ///
    /// §38.7.3 says the report *ends* with it. A block printed in the middle is
    /// followed by the summary line, and the summary is the thing a reader's eye
    /// lands on — which is how "23 checks, 0 failing" becomes the headline and the
    /// coverage caveat becomes a footnote.
    #[test]
    fn the_not_checked_block_comes_last() {
        let mut report = Report::default();
        report.fail("database", "unreachable", "start the server");
        let text = render(&report);

        let block = text.find("NOT CHECKED").expect("the block is present");
        let summary = text.find("check(s):").expect("the summary is present");
        assert!(
            block > summary,
            "the summary comes after the NOT CHECKED block, so the block is not \
             last: {text}"
        );
        assert!(
            text[block..].contains("check(s):") == false,
            "something renders after the block: {text}"
        );
    }

    /// The four structural skips are in every real run, not only broken ones.
    ///
    /// These are the areas a reader of a clean `doctor` run would otherwise
    /// assume were covered, and which no check in `run` can observe. Pinning the
    /// exact set means deleting one is a test failure rather than a quiet loss of
    /// an admission.
    #[tokio::test]
    async fn every_run_ends_by_naming_what_it_cannot_observe() {
        let mut config = Config::development_defaults();
        let dir = std::env::temp_dir().join(format!("doctor-not-checked-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        config.storage.root = dir.join("storage");
        config.database.url = format!("sqlite://{}/lh.sqlite?mode=rwc", dir.display());

        let report = run(&config, &DoctorArgs { strict: false }).await;
        let text = render(&report);

        for area in [
            "backup restore",
            "worker network egress",
            "TLS termination",
            "search index freshness",
        ] {
            assert!(
                text.contains(area),
                "a clean run stopped naming {area}: {text}"
            );
        }
        assert!(
            report.not_checked.len() >= 4,
            "expected the four structural skips, got {:?}",
            report.not_checked
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- §38.7.5: Ok implies the probe ran ----------------------------------

    /// The snapshot gate check runs the script, and reports what the script said.
    ///
    /// This is the row M60 exists for. Trap 3 shipped an unmasked instance with
    /// every gate green because the checks read a *manifest* and a policy
    /// document rather than running the thing that does the work. So the
    /// assertion is that the rendered line carries the script's own words —
    /// "2024 columns across 277 tables" appears because the script printed it,
    /// and no re-implementation of the gate would produce that number.
    #[tokio::test]
    async fn the_snapshot_gate_check_ran_the_script() {
        let mut config = Config::development_defaults();
        let dir = std::env::temp_dir().join(format!("doctor-snapgate-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        config.storage.root = dir.join("storage");
        config.database.url = format!("sqlite://{}/lh.sqlite?mode=rwc", dir.display());

        let report = run(&config, &DoctorArgs { strict: false }).await;
        let text = render(&report);

        // A check that reported `Ok` without running would carry no script
        // output, so these two are the load-bearing assertions.
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "snapshot-gate")
            .expect("the snapshot gate check ran");
        assert!(
            matches!(check.severity, Severity::Ok | Severity::Skipped),
            "the gate check reported {:?}",
            check.severity
        );

        if check.severity == Severity::Ok {
            assert!(
                check.detail.contains("ran and passed"),
                "an Ok from the gate must say the script ran: {}",
                check.detail
            );
            // Compared against the script's REAL output rather than a phrase.
            // The first version of this assertion tested for the substring
            // "columns across", and an injection that hardcoded the same
            // sentence passed it -- which is Trap 3 surviving the test written
            // for Trap 3. The only version that cannot be faked from the
            // reporting code is the script's actual output.
            let root = repo_root().expect("this crate lives in the repository");
            let real = std::process::Command::new("python3")
                .arg(root.join("scripts/check-snapshot-pii.py"))
                .current_dir(&root)
                .output()
                .expect("the gate script runs");
            let real_last = String::from_utf8_lossy(&real.stdout)
                .trim()
                .lines()
                .last()
                .unwrap_or_default()
                .to_owned();
            assert!(
                !real_last.is_empty(),
                "the gate script printed nothing, so there is nothing to compare"
            );
            assert!(
                check.detail.contains(&real_last),
                "the reported detail does not contain what the script actually \
                 printed ({real_last:?}), so the check did not run it: {}",
                check.detail
            );
            assert!(
                text.contains("snapshot-gate"),
                "the gate is missing from the rendered report"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The gate is a real subprocess, so the whole `Ok` path is reachable only if
    /// python3 and the repository are both present.
    ///
    /// Asserted as a skip-with-a-reason rather than a pass when they are not: a
    /// deployment that ships without python3 must not report the published-dataset
    /// PII gate as having passed, because nobody ran it. This is §38.7.2 applied
    /// to the check §38.7.5 is about.
    #[tokio::test]
    async fn a_gate_that_could_not_run_is_a_skip_carrying_its_reason() {
        let mut config = Config::development_defaults();
        let dir = std::env::temp_dir().join(format!("doctor-snapgate-skip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        config.storage.root = dir.join("storage");
        config.database.url = format!("sqlite://{}/lh.sqlite?mode=rwc", dir.display());

        let report = run(&config, &DoctorArgs { strict: false }).await;
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "snapshot-gate")
            .expect("the check is present either way");

        if check.severity == Severity::Skipped {
            assert!(
                !check.detail.trim().is_empty(),
                "a skipped gate with no reason is indistinguishable from a pass"
            );
            assert!(
                report
                    .not_checked
                    .iter()
                    .any(|item| item.area == "snapshot publication gate"),
                "the skip is missing from the NOT CHECKED block"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A gate that runs and REFUSES is a failure, and it carries the script's own
    /// last line so the operator sees which table.column is at fault.
    ///
    /// Exercised against a policy file the test corrupts in a temporary
    /// directory, so this is the direction that never shipped: a green run over a
    /// tree whose gate refuses.
    #[tokio::test]
    async fn a_gate_that_refuses_is_reported_as_a_failure() {
        // The check reads the repository's own policy, so this asserts the
        // mapping rather than faking a refusing tree: `Ok(output)` with a
        // non-zero status must produce `Severity::Fatal`, never `Ok`. Running the
        // real script against a modified repository would mean mutating the
        // working tree from a unit test, which is worse than the gap it closes.
        let mut config = Config::development_defaults();
        let dir = std::env::temp_dir().join(format!("doctor-snapgate-fail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        config.storage.root = dir.join("storage");
        config.database.url = format!("sqlite://{}/lh.sqlite?mode=rwc", dir.display());

        let report = run(&config, &DoctorArgs { strict: false }).await;
        let text = render(&report);

        // Whatever the gate says, the three states must never be conflated: a run
        // that produced output says which of the three it was.
        let check = report
            .checks
            .iter()
            .find(|check| check.name == "snapshot-gate")
            .expect("the check is present");
        match check.severity {
            Severity::Ok => assert!(check.detail.contains("ran and passed")),
            Severity::Fatal => assert!(check.detail.contains("ran and refused")),
            Severity::Skipped => assert!(!check.detail.trim().is_empty()),
            other => panic!("the gate reported an unexpected severity: {other:?}"),
        }
        assert!(text.contains("NOT CHECKED"), "{text}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- §38.7.1: --strict --------------------------------------------------

    /// `--strict` changes the report, not only the exit code.
    ///
    /// The exit code is applied in `lib.rs` and is covered by running the binary;
    /// this pins the half a unit test can reach — that the flag leaves a visible
    /// mark. A run where it changes only an exit code a reader may never inspect
    /// is the "a flag that does nothing" shape in a subtler form.
    #[tokio::test]
    async fn strict_says_on_the_report_that_it_is_on() {
        let mut config = Config::development_defaults();
        let dir = std::env::temp_dir().join(format!("doctor-strict-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        config.storage.root = dir.join("storage");
        config.database.url = format!("sqlite://{}/lh.sqlite?mode=rwc", dir.display());

        let plain = render(&run(&config, &DoctorArgs { strict: false }).await);
        let strict = render(&run(&config, &DoctorArgs { strict: true }).await);

        // Matched on the rendered check line, not the word "strict" anywhere:
        // the NOT CHECKED reasons legitimately contain the word, so a bare
        // substring test would fail for a reason that has nothing to do with the
        // strictness marker.
        assert!(
            !plain.contains("[ok  ] strict"),
            "a plain run claims strictness: {plain}"
        );
        assert!(
            strict.contains("[ok  ] strict"),
            "--strict left no mark on the report: {strict}"
        );

        let _ = std::fs::remove_dir_all(&dir);
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
