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
use lorehaven_domain::AccountId;
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

    /// Whether this is a local development instance.
    ///
    /// Development-only affordances are gated on this rather than on a feature
    /// flag, so a route that exists to make the interface drivable before its
    /// real caller arrives cannot be reached in a deployment.
    #[must_use]
    pub const fn is_development(self) -> bool {
        matches!(self, Self::Development)
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
    /// Operator-only settings.
    pub administration: AdministrationConfig,
    /// Rate limits.
    pub rate_limits: crate::limiter::Limits,
    /// What the importer may do about a source that refuses a plain request.
    pub imports: ImportsConfig,
    /// Theme mode and gravity settings (spec §0.4.6).
    pub theme: ThemeConfig,
    /// Discovery feed diversity settings.
    pub discovery: DiscoveryConfig,
    /// TTS narration settings.
    pub tts: TtsConfig,
    /// Bulk export settings.
    pub bulk_export: BulkExportConfig,
    /// Forum settings (spec 35).
    pub forum: ForumConfig,
    /// Resource directory settings (spec §39).
    pub directory: DirectoryConfig,
    /// Export retention settings (spec §38).
    pub exports: ExportsConfig,
    /// Work permission statements and fork guards (spec §40).
    pub works: WorksConfig,
    /// Longevity signals: half-life and interaction warmth (spec §41).
    pub community: CommunityConfig,
    /// Taste gravity settings (spec §0.4, §16.17).
    pub taste: TasteConfig,
    /// Flexible bounty settings (spec §20.3.2).
    pub bounties: BountiesConfig,
    /// Signal weighting settings (spec §9.7.3).
    pub signals: SignalsConfig,
    /// Instance preset (spec §0.6).
    pub instance: InstanceConfig,
    /// Meta-ranker settings (spec §9.10).
    pub meta_ranker: MetaRankerConfig,
    /// Revision caching settings (spec §38).
    pub revisions: RevisionsConfig,
    /// Job queue settings (spec §38).
    pub jobs: JobsConfig,
    /// Library update-check settings (spec §38).
    pub library: LibraryConfig,
    /// Where the configuration file was read from, if any.
    pub config_path: Option<PathBuf>,
}

/// Work permission settings (spec §40).
#[derive(Debug, Clone)]
pub struct WorksConfig {
    /// Maximum fork chain depth. A work whose lineage chain is already this
    /// deep refuses to fork. Default 3.
    pub max_fork_depth: u32,
}

impl Default for WorksConfig {
    fn default() -> Self {
        Self {
            max_fork_depth: 3,
        }
    }
}


/// Operator-only settings.
///
/// **There is no staff model yet.** Milestone 5 needs somebody who may look at
/// the queue, and the trust model arrives in Milestone 13, so this names *one
/// account* — never a boolean, never a role, and never a column on `accounts`.
/// M13 replaces it with a trust level (see
/// `docs/plans/junior-implementation-plan.md` §M13), and a `is_admin` flag would
/// have survived until then and been wrong in every query that read it.
#[derive(Debug, Clone, Default)]
pub struct AdministrationConfig {
    /// The account allowed to reach `/admin` routes, if any.
    pub operator_account_id: Option<AccountId>,
    /// Webhook delivery settings.
    pub webhook_timeout_secs: u64,
    pub webhook_max_attempts: u32,
    pub webhook_base_delay_ms: u64,
    pub webhook_allowed_hosts: Vec<String>,
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

/// Who defines instance taste, in priority order (spec §0.4.6, §16.15).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InfluenceSourceConfig {
    /// The kind of influence source.
    pub kind: InfluenceSourceKind,
    /// Minimum number of distinct accounts required for cohort/role/long_term sources.
    #[serde(default = "default_min_members")]
    pub min_members: usize,
    /// For `long_term_users`: minimum account age in days.
    #[serde(default = "default_tenure_days")]
    pub min_tenure_days: u64,
    /// For `long_term_users`: minimum contribution events.
    #[serde(default = "default_min_contributions")]
    pub min_contributions: u64,
    /// For `roles`: which roles qualify.
    #[serde(default)]
    pub roles: Vec<String>,
    /// For `cohort`: explicit member pseud ids.
    #[serde(default)]
    pub members: Vec<String>,
}

fn default_min_members() -> usize {
    5
}
fn default_tenure_days() -> u64 {
    90
}
fn default_min_contributions() -> u64 {
    5
}

/// The kinds of influence sources (spec §16.15).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InfluenceSourceKind {
    /// The operator's declared public topics.
    OperatorTopics,
    /// The §16.2 administrator taste profile.
    AdminTaste,
    /// Aggregate of long-term contributors (§16.15 `long_term_users`).
    LongTermUsers,
}

/// Theme mode and gravity settings (spec §0.4.6).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ThemeConfig {
    /// The instance's discovery posture: `generic`, `thematic`, or `adaptive`.
    #[serde(default = "default_theme_mode")]
    pub mode: String,
    /// Whether the §16.5 dial can reach zero for theme influence.
    /// When false, `theme_dial_floor_bp` is the dial's lower bound.
    #[serde(default = "true_bool")]
    pub allow_user_opt_out: bool,
    /// Lower bound for the §16.5 dial when opt-out is locked (in basis points).
    #[serde(default = "default_theme_dial_floor_bp")]
    pub theme_dial_floor_bp: i64,
    /// Maximum adaptive drift in basis points (0 = no drift).
    #[serde(default)]
    pub adaptive_max_drift_bp: i64,
    /// Ordered influence sources that shape instance taste.
    #[serde(default = "default_influence_sources")]
    pub influence_sources: Vec<InfluenceSourceConfig>,
    /// Tag names that increase topic gravity when a work matches (case-insensitive).
    #[serde(default)]
    pub boost_tags: Vec<String>,
    /// Tag names that decrease topic gravity when a work matches (case-insensitive).
    #[serde(default)]
    pub suppress_tags: Vec<String>,
    /// Per-tag gravity overrides in basis points (tag -> bp).
    #[serde(default)]
    pub tag_gravity_bp: std::collections::HashMap<String, i64>,
}

fn default_theme_mode() -> String {
    "thematic".to_string()
}
fn default_theme_dial_floor_bp() -> i64 {
    1000
}
fn true_bool() -> bool {
    true
}
fn default_influence_sources() -> Vec<InfluenceSourceConfig> {
    vec![InfluenceSourceConfig {
        kind: InfluenceSourceKind::OperatorTopics,
        min_members: 5,
        min_tenure_days: 90,
        min_contributions: 5,
        roles: Vec::new(),
        members: Vec::new(),
    }]
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            mode: default_theme_mode(),
            allow_user_opt_out: true,
            theme_dial_floor_bp: default_theme_dial_floor_bp(),
            adaptive_max_drift_bp: 0,
            influence_sources: default_influence_sources(),
            boost_tags: Vec::new(),
            suppress_tags: Vec::new(),
            tag_gravity_bp: std::collections::HashMap::new(),
        }
    }
}

/// Discovery feed diversity settings.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryConfig {
    /// Maximum works from the same fandom in one feed response (0 = unlimited).
    pub per_fandom_cap: usize,
    /// Fraction of the feed reserved for exploration (new fandoms).
    pub exploration_rate: f64,
    /// Whether the half-life job runs and scores influence discovery ranking.
    pub enable_half_life: bool,
    /// Minimum age in days before a work is eligible for half-life scoring.
    pub half_life_min_age_days: i64,
    /// Window size in days for both recent and first-window reader counts.
    pub half_life_window_days: i64,
}

/// Community / interaction tier settings (spec §41.2).
#[derive(Debug, Clone)]
pub struct CommunityConfig {
    /// Warmth thresholds in basis points. Default: lurk 0, react 200,
    /// comment 1000, create 3000.
    pub warmth_thresholds: serde_json::Value,
}

impl Default for CommunityConfig {
    fn default() -> Self {
        Self {
            warmth_thresholds: serde_json::json!({
                "lurk": 0,
                "react": 200,
                "comment": 1000,
                "create": 3000
            }),
        }
    }
}

/// Flexible bounty settings (spec §20.3.2, M18 Phase 4.1).
#[derive(Debug, Clone)]
pub struct BountiesConfig {
    /// Allowed bounty types. Default: standard, crowdfunded, reverse.
    pub allowed_types: Vec<String>,
    /// Minimum bounty amount in credits.
    pub min_amount: i64,
    /// Maximum bounty amount in credits.
    pub max_amount: i64,
    /// Crowdfunded bounty auto-activate threshold (fraction 0.0..1.0).
    pub crowdfund_activation_threshold: f64,
}

impl Default for BountiesConfig {
    fn default() -> Self {
        Self {
            allowed_types: vec![
                "standard".to_string(),
                "crowdfunded".to_string(),
                "reverse".to_string(),
            ],
            min_amount: 10,
            max_amount: 10_000,
            crowdfund_activation_threshold: 1.0,
        }
    }
}

/// Taste gravity settings (spec §0.4, §16.17).
#[derive(Debug, Clone)]
pub struct TasteConfig {
    /// Taste gravity strength (0.0 = off, 1.0 = full influence).
    pub gravity_strength: f64,
    /// Signal weighting mode.
    pub signal_weight_mode: String,
    /// Admin engagement weight multiplier.
    pub admin_weight: f64,
    /// Diversity injection percentage (0.0..1.0, 0.0 = monoculture).
    pub diversity_injection_percent: f64,
    /// Taste dimensions (names only; vectors are computed at runtime).
    pub dimensions: Vec<String>,
}

impl Default for TasteConfig {
    fn default() -> Self {
        Self {
            gravity_strength: 0.0,
            signal_weight_mode: "taste_weighted".to_string(),
            admin_weight: 1.0,
            diversity_injection_percent: 0.1,
            dimensions: vec![
                "angst".to_string(),
                "pacing".to_string(),
                "prose_density".to_string(),
                "canon_compliance".to_string(),
                "trope_diversity".to_string(),
            ],
        }
    }
}

/// Signal weighting settings (spec §9.7.3).
#[derive(Debug, Clone)]
pub struct SignalsConfig {
    /// Signal weighting mode: egalitarian, taste_weighted, admin_only.
    pub mode: String,
    /// Diversity injection percentage (0.0..1.0).
    pub diversity_injection_percent: f64,
}

impl Default for SignalsConfig {
    fn default() -> Self {
        Self {
            mode: "taste_weighted".to_string(),
            diversity_injection_percent: 0.1,
        }
    }
}

/// Instance preset (spec §0.6).
#[derive(Debug, Clone)]
pub struct InstanceConfig {
    /// Preset name: open_library, curated_boutique, admin_garden, genre_haven, experimental_lab, custom.
    pub preset: String,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            preset: "curated_boutique".to_string(),
        }
    }
}

/// Meta-ranker (Thompson Sampling over recommendation strategies) settings (spec §9.10).
#[derive(Debug, Clone)]
pub struct MetaRankerConfig {
    pub enabled: bool,
    pub exploration_percent: u8,
    pub exploitation_percent: u8,
    pub min_impressions_per_strategy: u64,
    pub rebalance_frequency_hours: u64,
    pub max_active_strategies: usize,
    pub success_metric: String,
    pub auto_disable_threshold: f64,
    pub candidate_exploration_bonus: f64,
    pub candidate_promotion_impressions: u64,
}

impl Default for MetaRankerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            exploration_percent: 15,
            exploitation_percent: 85,
            min_impressions_per_strategy: 50,
            rebalance_frequency_hours: 24,
            max_active_strategies: 20,
            success_metric: "admin_aligned".to_string(),
            auto_disable_threshold: 0.3,
            candidate_exploration_bonus: 2.0,
            candidate_promotion_impressions: 500,
        }
    }
}

/// Revision caching settings (spec §38).
#[derive(Debug, Clone)]
pub struct RevisionsConfig {
    /// How long a cached source revision lives, in seconds.
    pub ttl_secs: i64,
}

impl Default for RevisionsConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 7 * 24 * 60 * 60,
        }
    }
}

/// Job queue settings (spec §38).
#[derive(Debug, Clone)]
pub struct JobsConfig {
    /// How long terminal jobs are kept after completion/failure, in days.
    pub terminal_retention_days: i64,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            terminal_retention_days: 30,
        }
    }
}

/// Library update-check settings (spec §38).
#[derive(Debug, Clone)]
pub struct LibraryConfig {
    /// How long an update-check record is kept, in days.
    pub update_check_retention_days: i64,
    /// How many items one library update job checks.
    pub check_batch: i64,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            update_check_retention_days: 90,
            check_batch: 50,
        }
    }
}

/// Resource directory settings (spec §39).
#[derive(Debug, Clone)]
pub struct DirectoryConfig {
    /// Extra categories beyond the seed set (§39.2). Operators extend the
    /// directory through the config file; nothing is hardcoded here.
    pub extra_categories: Vec<String>,
    /// How votes are weighted (§39.4).
    pub weighting: String,
    /// Trust multipliers per rung of the §19.1 ladder (TL0..TL6).
    pub trust_vote_weights: [f64; 7],
    /// Floor/ceiling the taste affinity maps onto.
    pub taste_floor: f64,
    pub taste_ceiling: f64,
}

impl DirectoryConfig {
    /// The full category set: seed categories plus configured extras.
    pub fn categories(&self) -> Vec<String> {
        let mut out: Vec<String> = lorehaven_domain::directory::SEED_CATEGORIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        for extra in &self.extra_categories {
            let e = extra.trim().to_lowercase();
            if !e.is_empty() && !out.contains(&e) {
                out.push(e);
            }
        }
        out
    }

    /// The parsed weighting mode.
    pub fn weighting_mode(&self) -> lorehaven_domain::directory::VoteWeighting {
        lorehaven_domain::directory::VoteWeighting::parse(&self.weighting)
            .unwrap_or(lorehaven_domain::directory::VoteWeighting::TrustAndTaste)
    }
}

impl Default for DirectoryConfig {
    fn default() -> Self {
        Self {
            extra_categories: Vec::new(),
            weighting: "trust_and_taste".to_owned(),
            trust_vote_weights: lorehaven_domain::directory::DEFAULT_TRUST_VOTE_WEIGHTS,
            taste_floor: lorehaven_domain::directory::DEFAULT_TASTE_FLOOR,
            taste_ceiling: lorehaven_domain::directory::DEFAULT_TASTE_CEILING,
        }
    }
}

/// TTS narration settings (M26 / spec §32.5).
///
/// Piper is the built-in local engine. Cloud adapters (ElevenLabs, AWS
/// Polly) are added later behind the same `TtsEngine` trait.
#[derive(Debug, Clone)]
pub struct BulkExportConfig {
    /// How many works a bulk export may bundle. Default 50.
    pub max_items: i64,
    /// How many bytes a bulk export may total. Default 1073741824 (1 GiB).
    pub max_bytes: i64,
}

/// Forum settings (spec 35).
#[derive(Debug, Clone)]
pub struct ForumConfig {
    /// The discussion mode applied to **new** works. Existing works keep
    /// their own mode; the default is never applied retroactively.
    pub work_discussion_default: lorehaven_domain::work_discussion::WorkDiscussionMode,
    /// `(minimum trust level, votes per rolling 24h)` rungs of the vote
    /// budget, ascending by level (spec §35.2). A level below every rung gets
    /// the first rung's allowance.
    pub vote_budget: Vec<lorehaven_domain::typed_votes::BudgetRung>,
    /// Meta-mod points (vote flags) a TL4+ steward may spend per rolling 24h.
    pub meta_mod_points: i64,
    /// Verdicts a caster needs before their vote weight may decay. Below this
    /// a single flag changes nothing.
    pub meta_mod_min_verdicts: i64,
    /// The floor a decayed vote weight never goes below, in basis points.
    /// Lower weight, never fewer rights (spec §35.2).
    pub min_vote_weight_bp: i64,
    /// Monthly karma decay for an inactive receiver, in percent (§35.2).
    pub karma_decay_percent: i64,
}

impl Default for ForumConfig {
    fn default() -> Self {
        Self {
            work_discussion_default:
                lorehaven_domain::work_discussion::WorkDiscussionMode::CommentsOnly,
            // Spec §35.2: TL1=10, TL3=30, TL5=60.
            vote_budget: vec![(1, 10), (3, 30), (5, 60)],
            meta_mod_points: 20,
            meta_mod_min_verdicts: 3,
            min_vote_weight_bp: 100,
            karma_decay_percent: lorehaven_domain::typed_votes::KARMA_DECAY_PERCENT,
        }
    }
}

impl Default for BulkExportConfig {
    fn default() -> Self {
        Self {
            max_items: 50,
            max_bytes: 1073741824, // 1 GiB
        }
    }
}

/// TTS narration settings (M26 / spec §32.5).
///
/// Piper is the built-in local engine. Cloud adapters (ElevenLabs, AWS
/// Polly) are added later behind the same `TtsEngine` trait.
#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// Which engine to use. `"piper"` is the only built-in option.
    pub engine: String,
    /// Path to the `piper` binary. Defaults to `piper` on `PATH`.
    pub piper_path: Option<PathBuf>,
    /// Path to the Piper voice model (`.onnx`).
    pub piper_voice_model: Option<PathBuf>,
    /// Default voice name (maps to a Piper model).
    pub default_voice: Option<String>,
    /// Per-instance monthly spend cap in cents (cloud engines only).
    pub monthly_spend_cap_cents: Option<u64>,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            // Local-first is the decision (spec §38.2, lorehaven.toml.example):
            // an instance narrates with the binary on its own machine rather
            // than sending a reader's text to a service. `silent` remains a
            // valid explicit choice for hosts with no synthesizer.
            engine: "piper".into(),
            piper_path: None,
            piper_voice_model: None,
            default_voice: None,
            monthly_spend_cap_cents: None,
        }
    }
}

/// Export retention settings (spec §38).
#[derive(Debug, Clone)]
pub struct ExportsConfig {
    /// How long an export's output is kept after it is produced, in days.
    /// `0` means "keep forever" — the sweep never removes exports.
    pub retention_days: i64,
    /// How long a download grant lives, in seconds. Short by design: the reader
    /// who owns the export can always be given another one, so a leaked URL
    /// has a small window.
    pub grant_ttl_secs: i64,
    /// CTA placement in exported ebooks (spec §42): per-chapter (default),
    /// per-work (last chapter only), or off.
    pub cta_placement: String,
    /// The CTA HTML, sanitized by the instance. The reader does not set this.
    pub cta_html: String,
    /// Quorum of agreeing curator marks before a work is exempt from the
    /// instance CTA (spec §42.2).
    pub cta_quorum: i64,
}

impl Default for ExportsConfig {
    fn default() -> Self {
        Self {
            retention_days: 0,
            grant_ttl_secs: 3600,
            cta_placement: "per_chapter".to_string(),
            cta_html: "<p>If you enjoyed this work, show the author some love — <strong>leave a comment</strong>, <strong>share it</strong>, or <strong>start a discussion</strong>.</p>".to_string(),
            cta_quorum: 3,
        }
    }
}


///
/// # Why this is a configuration section and not a default
///
/// Each of these makes a request the source did not simply serve. A browser
/// fingerprint is a request that does not identify itself as Lorehaven. A solver
/// is a browser somebody runs on this instance's behalf. An archived copy is
/// somebody else's copy of the page rather than the page. An instance that did
/// all three for every challenging source would be doing things its operator
/// never agreed to, so the section is empty by default and every escalation has
/// to be switched on here.
///
/// The *other* half — which sources are behind a wall at all — belongs to the
/// adapter, which is the code that knows its site. This section supplies what the
/// instance is willing to run; the adapter supplies the need. Neither can grant
/// the other's half, which is why the two are merged in
/// [`ImportsConfig::unblock_for`] rather than either being the whole answer.
#[derive(Debug, Clone)]
pub struct ImportsConfig {
    /// A FlareSolverr-compatible service this instance may drive.
    ///
    /// The protocol rather than a program: FlareSolverr, Byparr and
    /// obscura-solverr all speak it, so pointing this at any of them works and
    /// changing tools is a configuration edit. Normally a container on loopback,
    /// which is why a private address is not merely allowed here but the
    /// expected case.
    pub solver_url: Option<String>,
    /// Whether a page the source will not serve may be read from the Internet
    /// Archive instead.
    ///
    /// Off by default. It is the one escalation that reads a *different*
    /// resource, and — for a work the source has deleted — it is also the one
    /// that can put a work back in front of readers that its author withdrew.
    /// That is a preservation decision an operator should make deliberately.
    pub archive_fallback: bool,
    /// Whether a path a source's `robots.txt` forbids is refused.
    ///
    /// **On by default.** `robots.txt` is how a host states which of its pages
    /// it wants crawled, and a crawler that reads the file and then ignores it
    /// is the thing the file exists to be told about. So the compliant
    /// behaviour is what an instance does without being asked, and switching
    /// this off is a deliberate edit with a name on it.
    ///
    /// # Why an operator may switch it off
    ///
    /// Because this is a self-hosted archiving platform, and there are archives
    /// whose `robots.txt` forbids the whole site while they serve a public
    /// reading view. Where such a host has invited the public to read something,
    /// an operator may conclude that a personal import for personal reading is
    /// not what the rule was aimed at — a judgement about *their* instance, made
    /// by the only person who can answer for it. That judgement is theirs to
    /// make, and this is where they make it.
    ///
    /// # What it does not switch
    ///
    /// **Pacing.** `Crawl-delay` from the same file, and the one-second floor
    /// beneath it, are still enforced. A permission question and a load question
    /// arrive in one file; answering the first differently says nothing about
    /// the second, and an instance that overrode the permission and then
    /// hammered the host would have turned a lost permission into a lost
    /// address.
    ///
    /// **Access control.** `robots.txt` is a crawling convention, not
    /// authentication. Nothing here reads a credential, defeats a login, or
    /// reaches a page the host's own code gates — spec §11.5's prohibition on
    /// circumventing access control is untouched, and a page behind an age gate
    /// or a challenge is still out of reach.
    ///
    /// **The record.** Every overridden path is counted on the fetcher, and the
    /// first one per host is logged at `warn` naming the host and the setting.
    /// An operator who switches this on is the one who has to say how much it
    /// cost, and the count is what answers that.
    pub honour_robots: bool,
}

impl Default for ImportsConfig {
    fn default() -> Self {
        Self {
            solver_url: None,
            archive_fallback: false,
            // Compliance, because the alternative is a crawler nobody asked
            // for. See the field documentation for what switching it off means.
            honour_robots: true,
        }
    }
}

impl ImportsConfig {
    /// The escalation chain for one source.
    ///
    /// The adapter's declaration plus what this instance is willing to run. An
    /// adapter asking for a fingerprint gets one only if the build carries the
    /// feature; an instance with a solver configured offers it only to a source
    /// that declared a wall.
    #[must_use]
    pub fn unblock_for(
        &self,
        adapter: &dyn lorehaven_scrapers::SourceAdapter,
    ) -> lorehaven_scrapers::Unblock {
        let mut unblock = adapter.unblock();
        if let Some(url) = &self.solver_url {
            unblock = unblock.with_solver(lorehaven_scrapers::SolverConfig::new(url.clone()));
        }
        if self.archive_fallback {
            unblock = unblock.with_archive();
        }
        unblock
    }
}

impl ImportsConfig {
    /// Why this source cannot be imported on this instance, when it cannot.
    ///
    /// # Why this is a refusal and not a fallback
    ///
    /// A source behind a wall needs something this instance either has or has
    /// not: a build carrying the fingerprint transport, or a solver service it can
    /// reach. Where it has not, there is no request worth making — every page
    /// would come back a challenge, and the import would fail one page at a time
    /// with a message about the wrong thing. So the answer is produced before
    /// anything is queued, and it names the fix.
    ///
    /// The same shape as the source-health refusal beside it in the routes: work
    /// that cannot succeed is refused while a reader is still looking at the page,
    /// rather than promised to a queue that will fail it later.
    ///
    /// Returned rather than logged because on a self-hosted instance the reader
    /// and the operator are the same person, and this is the one refusal they can
    /// act on themselves.
    #[must_use]
    pub fn unreachable_reason(
        &self,
        adapter: &dyn lorehaven_scrapers::SourceAdapter,
    ) -> Option<String> {
        let name = adapter.display_name();
        match adapter.wall() {
            lorehaven_scrapers::Wall::None => None,
            lorehaven_scrapers::Wall::Fingerprint => (!lorehaven_scrapers::FINGERPRINT_SUPPORTED)
                .then(|| {
                    format!(
                        "the {name} source refuses a plain request and needs a browser's TLS fingerprint, which this build \
                         was compiled without; rebuild with the `cloudflare-impersonation` feature to read it"
                    )
                }),
            lorehaven_scrapers::Wall::Solver => self.solver_url.is_none().then(|| {
                format!(
                    "the {name} source answers a bot challenge that only a driven browser can clear, and this instance has \
                     no solver service configured; run FlareSolverr, Byparr or obscura-solverr and point \
                     `imports.solver_url` at it"
                )
            }),
        }
    }
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
    /// Instance topics (M30) — configurable public/private with bonus credits.
    pub topics: Vec<InstanceTopic>,
}

/// A topic an instance is about (M30 / spec §0.4).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct InstanceTopic {
    /// The operator's own label ("hurt/comfort", "omegaverse", ...).
    pub name: String,
    /// Whether the topic is shown publicly (landing page, meta API, leaderboards).
    #[serde(default)]
    pub public: bool,
    /// Extra credits a reader earns for finishing a work in this topic.
    #[serde(default = "default_bonus_credits")]
    pub bonus_credits: u32,
}

fn default_bonus_credits() -> u32 {
    0
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
    /// Where the secret key lives, when it is not in `LOREHAVEN_SECRET_KEY`.
    pub secret_key_file: Option<PathBuf>,
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
            topics: site_file.topics.unwrap_or_default(),
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
            secret_key_file: security_file.secret_key_file,
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
        // The operator: an argument, then the environment (clap resolves those
        // two), then the file. A malformed id is a startup error rather than a
        // silently missing operator, because the failure mode of the latter is
        // an admin page nobody can open.
        let administration_file = file.administration.clone().unwrap_or_default();
        let operator_account_id = global
            .operator_account_id
            .clone()
            .or(administration_file.operator_account_id)
            .map(|raw| {
                raw.parse::<AccountId>()
                    .with_context(|| format!("the operator account id {raw:?} is not a UUID"))
            })
            .transpose()?;

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
                export: build(
                    section.export_burst,
                    section.export_per_minute,
                    defaults.export,
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

        let imports_file = file.imports.unwrap_or_default();
        let imports = ImportsConfig {
            // An empty string in a config file means "unset", not "the solver is
            // the empty URL": the latter would fail at the first challenged
            // chapter instead of at startup.
            solver_url: imports_file
                .solver_url
                .map(|url| url.trim().to_owned())
                .filter(|url| !url.is_empty()),
            archive_fallback: imports_file.archive_fallback.unwrap_or(false),
            // Compliance unless the file says otherwise. `unwrap_or(true)`
            // rather than `ImportsConfig::default()`'s value read back, because
            // this is the line that decides it and it should read that way.
            honour_robots: imports_file.honour_robots.unwrap_or(true),
        };

        // --- tts ------------------------------------------------------------
        let tts_file = file.tts.unwrap_or_default();
        let tts = TtsConfig {
            // An empty string means "unset", not "an engine named """, for the
            // same reason an empty solver URL does: the failure belongs at the
            // first narration, where the message can name what to install.
            engine: tts_file
                .engine
                .map(|engine| engine.trim().to_owned())
                .filter(|engine| !engine.is_empty())
                .unwrap_or_else(|| TtsConfig::default().engine),
            piper_path: tts_file.piper_path,
            piper_voice_model: tts_file.piper_voice_model,
            default_voice: tts_file
                .default_voice
                .map(|voice| voice.trim().to_owned())
                .filter(|voice| !voice.is_empty()),
            monthly_spend_cap_cents: tts_file.monthly_spend_cap_cents,
        };

        // --- forum ----------------------------------------------------------
        let forum_file = file.forum.unwrap_or_default();
        let mut vote_budget: Vec<lorehaven_domain::typed_votes::BudgetRung> = forum_file
            .vote_budget
            .map(|rungs| {
                rungs
                    .iter()
                    .map(|(level, votes)| {
                        let level: i64 = level.trim().parse().map_err(|_| {
                            anyhow::anyhow!(
                                "forum.vote_budget key {level:?} is not a trust level"
                            )
                        })?;
                        Ok((level, *votes))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_else(|| ForumConfig::default().vote_budget);
        // Ascending by level, so resolution is "the highest rung reached" and
        // not "whichever key the parser happened to see last".
        vote_budget.sort_unstable_by_key(|(level, _)| *level);
        let forum = ForumConfig {
            work_discussion_default: forum_file
                .work_discussion_default
                .as_deref()
                .and_then(lorehaven_domain::work_discussion::WorkDiscussionMode::parse)
                .unwrap_or_else(|| ForumConfig::default().work_discussion_default),
            vote_budget,
            meta_mod_points: forum_file
                .meta_mod_points
                .unwrap_or_else(|| ForumConfig::default().meta_mod_points),
            meta_mod_min_verdicts: forum_file
                .meta_mod_min_verdicts
                .unwrap_or_else(|| ForumConfig::default().meta_mod_min_verdicts),
            min_vote_weight_bp: forum_file
                .min_vote_weight_bp
                .unwrap_or_else(|| ForumConfig::default().min_vote_weight_bp),
            karma_decay_percent: forum_file
                .karma_decay_percent
                .unwrap_or_else(|| ForumConfig::default().karma_decay_percent),
        };

        // --- exports ------------------------------------------------------
        let exports_file = file.exports.unwrap_or_default();
        let exports = ExportsConfig {
            retention_days: exports_file.retention_days.unwrap_or_else(|| ExportsConfig::default().retention_days),
            grant_ttl_secs: exports_file.grant_ttl_secs.unwrap_or_else(|| ExportsConfig::default().grant_ttl_secs),
            cta_placement: exports_file.cta_placement.unwrap_or_else(|| ExportsConfig::default().cta_placement),
            cta_html: exports_file.cta_html.unwrap_or_else(|| ExportsConfig::default().cta_html),
            cta_quorum: exports_file.cta_quorum.unwrap_or_else(|| ExportsConfig::default().cta_quorum),
        };

        // --- directory -----------------------------------------------------
        let directory = match file.directory {
            Some(d) => {
                let defaults = DirectoryConfig::default();
                let mut weights = defaults.trust_vote_weights;
                if let Some(w) = d.trust_vote_weights {
                    if w.len() == 7 {
                        weights = w.try_into().expect("seven weights");
                    }
                }
                DirectoryConfig {
                    extra_categories: d.extra_categories.unwrap_or_default(),
                    weighting: d.weighting.unwrap_or(defaults.weighting),
                    trust_vote_weights: weights,
                    taste_floor: d.taste_floor.unwrap_or(defaults.taste_floor),
                    taste_ceiling: d.taste_ceiling.unwrap_or(defaults.taste_ceiling),
                }
            }
            None => DirectoryConfig::default(),
        };

        // --- works (spec §40) ----------------------------------------------
        let works = WorksConfig {
            max_fork_depth: file.works.as_ref().and_then(|w| w.max_fork_depth).unwrap_or(3),
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
            administration: AdministrationConfig {
                operator_account_id,
                webhook_timeout_secs: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_timeout_secs)
                    .unwrap_or(10),
                webhook_max_attempts: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_max_attempts)
                    .unwrap_or(5),
                webhook_base_delay_ms: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_base_delay_ms)
                    .unwrap_or(500),
                webhook_allowed_hosts: file
                    .administration
                    .as_ref()
                    .and_then(|a| a.webhook_allowed_hosts.clone())
                    .unwrap_or_default(),
            },
            dev,
            accounts,
            age,
            rate_limits,
            imports,
            theme: {
                let t = file.theme.unwrap_or_default();
                ThemeConfig {
                    mode: t.mode.unwrap_or_else(default_theme_mode),
                    allow_user_opt_out: t.allow_user_opt_out.unwrap_or(true),
                    theme_dial_floor_bp: t.theme_dial_floor_bp.unwrap_or_else(default_theme_dial_floor_bp),
                    adaptive_max_drift_bp: t.adaptive_max_drift_bp.unwrap_or(0),
                    influence_sources: t.influence_sources.unwrap_or_else(default_influence_sources),
            boost_tags: t.boost_tags.unwrap_or_default(),
            suppress_tags: t.suppress_tags.unwrap_or_default(),
            tag_gravity_bp: t.tag_gravity_bp.unwrap_or_default(),
                }
            },
            discovery: DiscoveryConfig::default(),
            tts,
            // --- bulk_export (spec §38) -----------------------------------------
            bulk_export: BulkExportConfig {
                max_items: file
                    .bulk_export
                    .as_ref()
                    .and_then(|b| b.max_items)
                    .unwrap_or_else(|| BulkExportConfig::default().max_items),
                max_bytes: file
                    .bulk_export
                    .as_ref()
                    .and_then(|b| b.max_bytes)
                    .unwrap_or_else(|| BulkExportConfig::default().max_bytes),
            },
            forum,
            exports,
            directory,
            works,
            community: CommunityConfig::default(),
            bounties: {
                let b = file.bounties.clone().unwrap_or_default();
                BountiesConfig {
                    allowed_types: match b.allowed_types {
                        Some(ref types) if !types.is_empty() => types.clone(),
                        _ => BountiesConfig::default().allowed_types,
                    },
                    min_amount: b.min_amount.unwrap_or(BountiesConfig::default().min_amount),
                    max_amount: b.max_amount.unwrap_or(BountiesConfig::default().max_amount),
                    crowdfund_activation_threshold: b
                        .crowdfund_activation_threshold
                        .unwrap_or(BountiesConfig::default().crowdfund_activation_threshold),
                }
            },
            taste: {
                let t = file.taste.unwrap_or_default();
                TasteConfig {
                    gravity_strength: t.gravity_strength.unwrap_or(0.0),
                    signal_weight_mode: t.signal_weight_mode.unwrap_or_else(|| "taste_weighted".to_string()),
                    admin_weight: t.admin_weight.unwrap_or(1.0),
                    diversity_injection_percent: t.diversity_injection_percent.unwrap_or(0.1),
                    dimensions: t.dimensions.unwrap_or_else(|| vec![
                        "angst".to_string(),
                        "pacing".to_string(),
                        "prose_density".to_string(),
                        "canon_compliance".to_string(),
                        "trope_diversity".to_string(),
                    ]),
                }
            },
            signals: {
                let s = file.signals.unwrap_or_default();
                SignalsConfig {
                    mode: s.mode.unwrap_or_else(|| "taste_weighted".to_string()),
                    diversity_injection_percent: s.diversity_injection_percent.unwrap_or(0.1),
                }
            },
            instance: {
                let i = file.instance.unwrap_or_default();
                InstanceConfig {
                    preset: i.preset.unwrap_or_else(|| "curated_boutique".to_string()),
                }
            },
            // --- meta_ranker (spec §9.10) --------------------------------------
            meta_ranker: {
                let m = file.meta_ranker.unwrap_or_default();
                MetaRankerConfig {
                    enabled: m.enabled.unwrap_or(true),
                    exploration_percent: m.exploration_percent.unwrap_or(15) as u8,
                    exploitation_percent: m.exploitation_percent.unwrap_or(85) as u8,
                    min_impressions_per_strategy: m.min_impressions_per_strategy.unwrap_or(50),
                    rebalance_frequency_hours: m.rebalance_frequency_hours.unwrap_or(24),
                    max_active_strategies: m.max_active_strategies.unwrap_or(20),
                    success_metric: m
                        .success_metric
                        .unwrap_or_else(|| "admin_aligned".to_string()),
                    auto_disable_threshold: m.auto_disable_threshold.unwrap_or(0.3),
                    candidate_exploration_bonus: m.candidate_exploration_bonus.unwrap_or(2.0),
                    candidate_promotion_impressions: m.candidate_promotion_impressions.unwrap_or(500),
                }
            },
            // --- revisions (spec §38) -----------------------------------------
            revisions: RevisionsConfig {
                ttl_secs: file
                    .revisions
                    .as_ref()
                    .and_then(|r| r.ttl_secs)
                    .unwrap_or_else(|| RevisionsConfig::default().ttl_secs),
            },
            // --- jobs (spec §38) ----------------------------------------------
            jobs: JobsConfig {
                terminal_retention_days: file
                    .jobs
                    .as_ref()
                    .and_then(|j| j.terminal_retention_days)
                    .unwrap_or_else(|| JobsConfig::default().terminal_retention_days),
            },
            // --- library (spec §38) --------------------------------------------
            library: LibraryConfig {
                update_check_retention_days: file
                    .library
                    .as_ref()
                    .and_then(|l| l.update_check_retention_days)
                    .unwrap_or_else(|| LibraryConfig::default().update_check_retention_days),
                check_batch: file
                    .library
                    .as_ref()
                    .and_then(|l| l.check_batch)
                    .unwrap_or_else(|| LibraryConfig::default().check_batch),
            },
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
                topics: Vec::new(),
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
                secret_key_file: None,
                session_ttl: Duration::from_secs(60 * 60 * 24 * 30),
                csrf_required: true,
                trust_proxy: false,
            },
            logging: LoggingConfig {
                filter: "info".to_owned(),
                format: LogFormat::Pretty,
            },
            assets: AssetsConfig { dir: None },
            administration: AdministrationConfig {
                operator_account_id: None,
                webhook_timeout_secs: 10,
                webhook_max_attempts: 5,
                webhook_base_delay_ms: 500,
                webhook_allowed_hosts: Vec::new(),
            },
            dev: DevConfig { seed_enabled: true },
            accounts: AccountsConfig {
                registration_open: true,
            },
            age: AgeConfig {
                threshold: 14,
                guardian_workflow_enabled: false,
            },
            rate_limits: crate::limiter::Limits::default(),
            // Nothing is escalated to in tests. An instance that impersonated or
            // drove a solver by default would make every test that fetches a page
            // depend on which escalations happened to be configured.
            imports: ImportsConfig::default(),
            theme: ThemeConfig::default(),
            discovery: DiscoveryConfig::default(),
            tts: TtsConfig::default(),
            bulk_export: BulkExportConfig::default(),
            forum: ForumConfig::default(),
            exports: ExportsConfig::default(),
            directory: DirectoryConfig::default(),
            works: WorksConfig::default(),
            community: CommunityConfig::default(),
            bounties: BountiesConfig::default(),
            taste: TasteConfig::default(),
            signals: SignalsConfig::default(),
            instance: InstanceConfig::default(),
            meta_ranker: MetaRankerConfig::default(),
            revisions: RevisionsConfig::default(),
            jobs: JobsConfig::default(),
            library: LibraryConfig::default(),
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
        // Forum vote settings (spec §35.2). These are numbers a route uses to
        // refuse a vote, so a nonsense value has to stop the instance rather
        // than make every vote impossible or free.
        if self.forum.vote_budget.is_empty() {
            anyhow::bail!("forum.vote_budget must have at least one trust rung");
        }
        for (level, votes) in &self.forum.vote_budget {
            if *level < 0 {
                anyhow::bail!("forum.vote_budget trust levels must not be negative, got {level}");
            }
            if *votes < 0 {
                anyhow::bail!("forum.vote_budget must not be negative, got {votes} at level {level}");
            }
        }
        if self.forum.meta_mod_points < 0 {
            anyhow::bail!("forum.meta_mod_points must not be negative");
        }
        if self.forum.meta_mod_min_verdicts < 1 {
            anyhow::bail!("forum.meta_mod_min_verdicts must be at least 1");
        }
        if !(0..=lorehaven_domain::typed_votes::WEIGHT_SCALE_BP).contains(&self.forum.min_vote_weight_bp)
        {
            anyhow::bail!(
                "forum.min_vote_weight_bp must be between 0 and {}",
                lorehaven_domain::typed_votes::WEIGHT_SCALE_BP
            );
        }
        if !(0..=100).contains(&self.forum.karma_decay_percent) {
            anyhow::bail!("forum.karma_decay_percent must be between 0 and 100");
        }
        // Export retention settings (spec §38).
        if self.exports.retention_days < 0 {
            anyhow::bail!("exports.retention_days must be >= 0, got {}", self.exports.retention_days);
        }
        if self.exports.grant_ttl_secs <= 0 {
            anyhow::bail!("exports.grant_ttl_secs must be > 0, got {}", self.exports.grant_ttl_secs);
        }
        let cta_placement = &self.exports.cta_placement;
        if cta_placement != "per_chapter" && cta_placement != "per_work" && cta_placement != "off" {
            anyhow::bail!(
                "exports.cta_placement must be per_chapter, per_work, or off, got {}",
                cta_placement
            );
        }
        if self.exports.cta_quorum < 0 {
            anyhow::bail!(
                "exports.cta_quorum must be >= 0, got {}",
                self.exports.cta_quorum
            );
        }
        // Checked here rather than at the first challenged chapter: a solver URL
        // that does not parse is an operator's typo, and a typo should stop the
        // instance rather than surface hours later as an import that cannot read
        // one source.
        if let Some(url) = &self.imports.solver_url {
            let parsed = url::Url::parse(url)
                .map_err(|error| anyhow::anyhow!("imports.solver_url is not a URL: {error}"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                anyhow::bail!(
                    "imports.solver_url must be http or https, got {:?}",
                    parsed.scheme()
                );
            }
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
    administration: Option<AdministrationSection>,
    dev: Option<DevSection>,
    accounts: Option<AccountsSection>,
    age: Option<AgeSection>,
    rate_limits: Option<RateLimitSection>,
    imports: Option<ImportsSection>,
    tts: Option<TtsSection>,
    bulk_export: Option<BulkExportSection>,
    forum: Option<ForumSection>,
    exports: Option<ExportsSection>,
    directory: Option<DirectorySection>,
    works: Option<WorksSection>,
    revisions: Option<RevisionsSection>,
    jobs: Option<JobsSection>,
    library: Option<LibrarySection>,
    theme: Option<ThemeSection>,
    /// Taste gravity settings (spec §0.4, §16.17).
    taste: Option<TasteSection>,
    /// Flexible bounty settings (spec §20.3.2).
    bounties: Option<BountiesSection>,
    /// Signal weighting settings (spec §9.7.3).
    signals: Option<SignalsSection>,
    /// Instance preset (spec §0.6).
    instance: Option<InstanceSection>,
    /// Meta-ranker settings (spec §9.10).
    meta_ranker: Option<MetaRankerSection>,
}

/// The `[theme]` table (spec §0.4.6): theme mode and gravity settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeSection {
    mode: Option<String>,
    allow_user_opt_out: Option<bool>,
    theme_dial_floor_bp: Option<i64>,
    adaptive_max_drift_bp: Option<i64>,
    influence_sources: Option<Vec<InfluenceSourceConfig>>,
    boost_tags: Option<Vec<String>>,
    suppress_tags: Option<Vec<String>>,
    tag_gravity_bp: Option<std::collections::HashMap<String, i64>>,
}

/// The `[taste]` table (spec §0.4, §16.17): taste gravity settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TasteSection {
    gravity_strength: Option<f64>,
    signal_weight_mode: Option<String>,
    admin_weight: Option<f64>,
    diversity_injection_percent: Option<f64>,
    dimensions: Option<Vec<String>>,
}

/// The `[bounties]` table (spec §20.3.2): flexible bounty settings.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BountiesSection {
    allowed_types: Option<Vec<String>>,
    min_amount: Option<i64>,
    max_amount: Option<i64>,
    crowdfund_activation_threshold: Option<f64>,
}

/// The `[signals]` table (spec §9.7.3): signal weighting settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalsSection {
    mode: Option<String>,
    diversity_injection_percent: Option<f64>,
}

/// The `[instance]` table (spec §0.6): instance preset.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceSection {
    preset: Option<String>,
}

/// The `[meta_ranker]` table (spec §9.10): meta-ranker settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaRankerSection {
    /// Enable the multi-armed bandit meta-ranker.
    enabled: Option<bool>,
    /// % of discovery slots filled by randomly selected strategy.
    exploration_percent: Option<u32>,
    /// % filled by current best-ranked strategy.
    exploitation_percent: Option<u32>,
    /// Don't rank strategy until it has enough impressions.
    min_impressions_per_strategy: Option<u64>,
    /// How often strategy weights are recomputed, in hours.
    rebalance_frequency_hours: Option<u64>,
    /// Maximum number of active strategies.
    max_active_strategies: Option<usize>,
    /// Success metric: admin_aligned, engagement, completion, hybrid.
    success_metric: Option<String>,
    /// Auto-disable threshold (success_rate < threshold for N periods).
    auto_disable_threshold: Option<f64>,
    /// Bonus multiplier for candidate strategies during exploration.
    candidate_exploration_bonus: Option<f64>,
    /// Impressions before a candidate can be promoted.
    candidate_promotion_impressions: Option<u64>,
}

/// The `[revisions]` table (spec §38): source revision cache settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionsSection {
    /// How long a cached source revision lives, in seconds. Default 604800 (7d).
    ttl_secs: Option<i64>,
}

/// The `[jobs]` table (spec §38): job queue settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct JobsSection {
    /// How long terminal jobs are kept, in days. Default 30.
    terminal_retention_days: Option<i64>,
}

/// The `[library]` table (spec §38): library update-check settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibrarySection {
    /// How long an update-check record is kept, in days. Default 90.
    update_check_retention_days: Option<i64>,
    /// How many items one library update job checks. Default 50.
    check_batch: Option<i64>,
}

/// The `[bulk_export]` table (spec §38): bulk export limits.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkExportSection {
    /// How many works a bulk export may bundle. Default 50.
    max_items: Option<i64>,
    /// How many bytes a bulk export may total. Default 1073741824 (1 GiB).
    max_bytes: Option<i64>,
}

/// The `[works]` table (spec §40): fork and permission statement settings.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorksSection {
    /// Maximum fork chain depth (default 3).
    max_fork_depth: Option<u32>,
}

/// The `[tts]` table: which engine narrates, and how an operator configured it
/// (M26 / spec §32.5).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TtsSection {
    /// `piper` (local, the default) or `silent` (a pipeline check that needs no
    /// synthesizer). Cloud engines will be added behind the same `TtsEngine`.
    engine: Option<String>,
    /// Path to the `piper` binary. Unset means "whatever `piper` is on `PATH`".
    piper_path: Option<PathBuf>,
    /// Path to the Piper voice model (`.onnx`).
    piper_voice_model: Option<PathBuf>,
    /// The voice a narration uses when the request does not name one.
    default_voice: Option<String>,
    /// Per-instance monthly spend cap in cents. Only cloud engines can spend;
    /// this exists so the ceiling is configuration before an adapter that
    /// charges arrives, not a number invented in a route.
    monthly_spend_cap_cents: Option<u64>,
}

/// The `[forum]` table (spec 35.0).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForumSection {
    /// `thread_only` (recommended), `comments_only` (legacy default), or `both`.
    work_discussion_default: Option<String>,
    /// Vote budget rungs, keyed by trust level: `vote_budget = { "1" = 10,
    /// "3" = 30, "5" = 60 }` (spec §35.2).
    vote_budget: Option<std::collections::BTreeMap<String, i64>>,
    /// Meta-mod points a TL4+ steward may spend per rolling 24h.
    meta_mod_points: Option<i64>,
    /// Verdicts needed before a caster's vote weight may decay.
    meta_mod_min_verdicts: Option<i64>,
    /// The floor a decayed vote weight never goes below, in basis points.
    min_vote_weight_bp: Option<i64>,
    /// Monthly karma decay for an inactive receiver, in percent.
    karma_decay_percent: Option<i64>,
}

/// The `[exports]` table: how long an export's output is kept, and how long a
/// download grant lives (spec §38).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportsSection {
    /// How long an export's output is kept after it is produced, in days.
    /// `0` means "keep forever" — the sweep never removes exports.
    retention_days: Option<i64>,
    /// How long a download grant lives, in seconds.
    grant_ttl_secs: Option<i64>,
    /// CTA placement: per_chapter (default), per_work, or off (spec §42).
    cta_placement: Option<String>,
    /// The CTA HTML, sanitized by the instance. The reader does not set this.
    cta_html: Option<String>,
    /// Quorum of agreeing curator marks before a work is exempt from the CTA.
    cta_quorum: Option<i64>,
}

/// `[directory]` — the resource directory (spec §39.2, §39.4).
#[derive(Debug, Default, Deserialize)]
struct DirectorySection {
    /// Extra categories beyond the seed set.
    extra_categories: Option<Vec<String>>,
    /// `flat`, `trust` or `trust_and_taste` (the default).
    weighting: Option<String>,
    /// Seven trust multipliers, TL0..TL6.
    trust_vote_weights: Option<Vec<f64>>,
    /// Floor/ceiling the taste affinity maps onto.
    taste_floor: Option<f64>,
    taste_ceiling: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportsSection {
    /// A FlareSolverr-compatible service, e.g. `http://127.0.0.1:8191`.
    solver_url: Option<String>,
    /// Whether an archived copy may be read as a last resort.
    archive_fallback: Option<bool>,
    /// Whether a path a source's `robots.txt` forbids is refused. Default true.
    honour_robots: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SiteSection {
    name: Option<String>,
    base_url: Option<String>,
    contact_email: Option<String>,
    topics: Option<Vec<InstanceTopic>>,
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
    /// Where the secret key lives, when it is not in the environment.
    secret_key_file: Option<PathBuf>,
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
    export_burst: Option<u32>,
    export_per_minute: Option<u32>,
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
#[derive(Clone)]
struct AdministrationSection {
    operator_account_id: Option<String>,
    /// Webhook delivery settings (spec §38).
    webhook_timeout_secs: Option<u64>,
    webhook_max_attempts: Option<u32>,
    webhook_base_delay_ms: Option<u64>,
    webhook_allowed_hosts: Option<Vec<String>>,
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

impl Config {
    /// Parse a TOML body directly into a `Config` — for tests that need to
    /// assert config values without building an entire scratch instance.
    pub fn parse_from_str(body: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-parse-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, body).expect("write config");
        Self::load(&GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        })
    }
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
    }

    #[cfg(test)]
    fn load_from(name: &str, body: &str) -> Result<Config> {
        let dir =
            std::env::temp_dir().join(format!("lorehaven-imports-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("lorehaven.toml");
        std::fs::write(&path, body).expect("write config");
        Config::load(&GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        })
    }

    #[test]
    fn nothing_is_escalated_to_unless_it_is_configured() {
        // The default has to be that an instance does nothing on a source's
        // behalf that its operator did not ask for: no solver, no archived copy,
        // and no fingerprint unless an adapter declares one.
        let config = Config::development_defaults();
        assert!(config.imports.solver_url.is_none());
        assert!(!config.imports.archive_fallback);
        // And it reads the rules it is asked to follow. The list above is of
        // things an instance must *not* do unasked; this is the one thing it
        // must do unasked.
        assert!(
            config.imports.honour_robots,
            "an instance complies with robots.txt unless its operator says otherwise"
        );
    }

    #[test]
    fn a_source_forbidding_what_an_adapter_needs_is_still_imported_when_the_operator_says_so() {
        // The override an operator reaches for when an archive's `robots.txt`
        // forbids the site while it serves a public reading view. Read from the
        // file rather than built by assignment, because the point is that an
        // operator can set it without a code change.
        let parsed = load_from(
            "robots",
            "environment = \"development\"\n\
             [imports]\n\
             honour_robots = false\n",
        )
        .expect("an operator may override a source's Disallow rules");
        assert!(!parsed.imports.honour_robots);

        // And the default holds when the section is present but silent about it.
        let parsed = load_from(
            "robots-default",
            "environment = \"development\"\n\
             [imports]\n\
             archive_fallback = true\n",
        )
        .expect("a partial imports section is valid");
        assert!(parsed.imports.honour_robots);
    }

    #[test]
    fn tts_defaults_to_local_piper() {
        // Local-first is the decision: an instance narrates with the binary on
        // its own machine rather than sending a reader's text to a service.
        let config = Config::development_defaults();
        assert_eq!(config.tts.engine, "piper");
        assert!(config.tts.piper_path.is_none());
        assert!(config.tts.piper_voice_model.is_none());
        assert!(
            config.tts.monthly_spend_cap_cents.is_none(),
            "no instance has a cloud budget until it configures one"
        );
    }

    #[test]
    fn a_tts_section_is_read_from_the_file() {
        let parsed = load_from(
            "tts",
            "environment = \"development\"\n\
             [tts]\n\
             engine = \"piper\"\n\
             piper_path = \"/opt/piper/piper\"\n\
             piper_voice_model = \"/opt/piper/en_US-lessac-medium.onnx\"\n\
             default_voice = \"en_US-lessac-medium\"\n",
        )
        .expect("a [tts] section parses");
        assert_eq!(parsed.tts.engine, "piper");
        assert_eq!(
            parsed.tts.piper_path.expect("path"),
            PathBuf::from("/opt/piper/piper")
        );
        assert_eq!(
            parsed.tts.piper_voice_model.expect("model"),
            PathBuf::from("/opt/piper/en_US-lessac-medium.onnx")
        );
        assert_eq!(
            parsed.tts.default_voice.as_deref(),
            Some("en_US-lessac-medium")
        );
    }

    #[test]
    fn an_empty_tts_engine_is_unset_rather_than_an_empty_name() {
        // `engine = ""` would otherwise become an engine the builder cannot
        // name, and the operator would meet the failure at the first narration
        // instead of at startup. The default is what an empty value means.
        let parsed = load_from(
            "tts-empty",
            "environment = \"development\"\n[tts]\nengine = \"  \"\n",
        )
        .expect("an empty engine is tolerated");
        assert_eq!(parsed.tts.engine, "piper");
    }

    #[test]
    fn an_unknown_key_in_the_tts_section_is_refused() {
        // `deny_unknown_fields`, like every other section: a typo in a config
        // file must be an error naming the file, not a setting that is silently
        // ignored.
        let error = load_from(
            "tts-typo",
            "environment = \"development\"\n[tts]\npiper_voice = \"x\"\n",
        )
        .expect_err("an unknown tts key is refused");
        assert!(
            error.to_string().contains("tts") || error.to_string().contains("unknown field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_shipped_example_configuration_parses() {
        // The example is what an operator copies, so a section it documents and
        // the parser does not accept is a startup failure handed to every new
        // instance. Every other config test writes its own file; this one reads
        // the one that ships.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lorehaven.toml.example");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("the example config must exist: {error}"));
        // `load` needs a real path, and the example is not writable in a
        // checkout, so it is parsed through the same entry point by copying it.
        let dir = std::env::temp_dir().join(format!("lorehaven-example-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("lorehaven.toml");
        std::fs::write(&target, &body).expect("copy the example");
        let parsed = Config::load(&GlobalArgs {
            config: Some(target),
            ..GlobalArgs::default()
        })
        .expect("the shipped example must be a configuration this build accepts");

        // Named settings the example documents, asserted so a rename in the
        // parser cannot leave the example describing something that no longer
        // exists — `deny_unknown_fields` would not catch a *comment*.
        assert!(parsed.imports.honour_robots);
        assert!(parsed.imports.solver_url.is_none());
        assert!(!parsed.imports.archive_fallback);
        // And the sections themselves: a section that exists only in the parser
        // leaves every operator copying the example without the setting.
        assert_eq!(parsed.tts.engine, "piper");
        assert!(parsed.tts.piper_path.is_none());
        for section in [
            "[site]",
            "[server]",
            "[database]",
            "[storage]",
            "[security]",
            "[logging]",
            "[assets]",
            "[imports]",
            "[tts]",
            "[forum]",
            "[dev]",
        ] {
            assert!(
                body.contains(section),
                "the example config must document {section}"
            );
        }
    }

    #[test]
    fn a_solver_url_is_read_from_the_config_file() {
        let parsed = load_from(
            "solver",
            "environment = \"development\"\n\
             [imports]\n\
             solver_url = \"http://127.0.0.1:8191\"\n\
             archive_fallback = true\n",
        )
        .expect("a solver on loopback is a valid configuration");
        assert_eq!(
            parsed.imports.solver_url.as_deref(),
            Some("http://127.0.0.1:8191")
        );
        assert!(parsed.imports.archive_fallback);
    }

    #[test]
    fn a_malformed_solver_url_stops_the_instance() {
        // A typo in an operator's config should stop the instance rather than
        // surface hours later as an import that cannot read one source.
        for (name, url) in [("typo", "not a url"), ("scheme", "ftp://solver")] {
            let body =
                format!("environment = \"development\"\n[imports]\nsolver_url = \"{url}\"\n");
            assert!(
                load_from(name, &body).is_err(),
                "a solver URL of {url:?} was accepted"
            );
        }
    }

    #[test]
    fn an_empty_solver_url_means_unset_rather_than_the_empty_url() {
        let parsed = load_from(
            "empty",
            "environment = \"development\"\n[imports]\nsolver_url = \"  \"\n",
        )
        .expect("an empty value is not an error");
        assert!(parsed.imports.solver_url.is_none());
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

    #[test]
    fn theme_defaults_to_thematic_with_operator_topics() {
        // Spec §0.4.6: the default is thematic — an instance that declares
        // topics is assumed to want them to matter.
        let config = Config::development_defaults();
        assert_eq!(config.theme.mode, "thematic");
        assert!(config.theme.allow_user_opt_out);
        assert_eq!(config.theme.theme_dial_floor_bp, 1000);
        assert_eq!(config.theme.adaptive_max_drift_bp, 0);
        assert_eq!(config.theme.influence_sources.len(), 1);
        assert_eq!(
            config.theme.influence_sources[0].kind,
            InfluenceSourceKind::OperatorTopics
        );
    }

    #[test]
    fn theme_section_is_parsed_from_toml() {
        let config = load_from(
            "theme",
            r#"
[theme]
mode = "generic"
allow_user_opt_out = false
theme_dial_floor_bp = 2500
adaptive_max_drift_bp = 500

[[theme.influence_sources]]
kind = "operator_topics"

[[theme.influence_sources]]
kind = "long_term_users"
min_tenure_days = 120
min_contributions = 8
"#,
        )
        .expect("loads");
        assert_eq!(config.theme.mode, "generic");
        assert!(!config.theme.allow_user_opt_out);
        assert_eq!(config.theme.theme_dial_floor_bp, 2500);
        assert_eq!(config.theme.adaptive_max_drift_bp, 500);
        assert_eq!(config.theme.influence_sources.len(), 2);
        assert_eq!(
            config.theme.influence_sources[0].kind,
            InfluenceSourceKind::OperatorTopics
        );
        assert_eq!(
            config.theme.influence_sources[1].kind,
            InfluenceSourceKind::LongTermUsers
        );
        assert_eq!(config.theme.influence_sources[1].min_tenure_days, 120);
        assert_eq!(config.theme.influence_sources[1].min_contributions, 8);
    }

    #[test]
    fn theme_unknown_mode_is_still_loaded_but_named() {
        // The mode is a string; an unknown value is not a parse error (the
        // operator may be on a newer spec), but the discovery route treats
        // anything it does not recognise as generic — no gravity.
        let config = load_from(
            "theme-unknown",
            r#"
[theme]
mode = "quantum"
"#,
        )
        .expect("loads");
        assert_eq!(config.theme.mode, "quantum");
    }
}
