//! §23.7's AI provider interface, as domain types.
//!
//! This is the layer that does not need a provider to exist, which is why it comes first
//! for gap C (§ `docs/plans/gap-c-ai-pre-read-scoring.md`). Everything here is a value,
//! a validation, or a trait signature — no I/O, no HTTP client, no configuration. A
//! caller can be written and tested against this module today, and an Ollama or
//! OpenAI-compatible adapter can be added later without changing any of it.
//!
//! Three requirements from §23.7 shape the types rather than decorating them:
//!
//! - **"disabled without configuration"** — [`AiProvider::available`] is a method, not a
//!   field, and every call site goes through it. A configured-but-empty provider and an
//!   unconfigured one are the same code path, so there is no "forgot to check" failure
//!   mode to test for.
//! - **"quoted costs"** — [`CostQuote`] is returned *before* the work runs and the
//!   charge comes back *after*, so a caller that would exceed its budget never has to
//!   cancel mid-flight.
//! - **"generated output distinguished from author text"** — [`AiTask`] is part of every
//!   response, and [`AiVerdict`] carries it, so output cannot be stored without recording
//!   that it is not the author's.
//!
//! **The central refusal: nothing here can reach a ranking signal.** [`PreReadVerdict`]
//! is a plain struct with no conversion into any ranking type, and no function in this
//! module takes a `ReaderSignals`, a quality multiplier, or a strategy context. §0.3
//! says no credit, payment or trust level can move any ranking signal; the cheapest way
//! to honour that is for the score to have no path to one at all.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::{AppError, Result};

/// The AI tasks §23.7 enumerates, plus pre-read scoring.
///
/// [`AiTask::PreReadScoring`] is not in §23.7's list. It is here because gap C needs it
/// and §23.7's "optional metadata suggestions" is the closest existing authorisation —
/// scoring an import against the operator's configured dimensions is metadata suggestion
/// about a work's own text, done before publication rather than after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AiTask {
    Translation,
    Summarization,
    GrammarAssistance,
    PromptAssistance,
    Embeddings,
    /// §23.7's "optional metadata suggestions".
    MetadataSuggestions,
    CommentClassification,
    SearchAssistance,
    /// Gap C. Author-facing, private, never a ranking input.
    PreReadScoring,
}

impl AiTask {
    /// Whether this task may see a reader's or author's **private** text.
    ///
    /// §23.7 requires "explicit private-text consent" for anything touching private text,
    /// and it is the difference between a task that needs a consent check and one that
    /// does not — so it is a property of the task, not a per-call argument a caller can
    /// forget to consult.
    pub fn touches_private_text(self) -> bool {
        matches!(
            self,
            AiTask::GrammarAssistance
                | AiTask::PromptAssistance
                | AiTask::CommentClassification
                | AiTask::PreReadScoring
        )
    }

    /// Whether this task's output may be attributed to the author or published as-is.
    ///
    /// §23.7: "no automatic publication, generated output distinguished from author
    /// text". Every task here is `false` — there is no task whose output may appear as
    /// the author's words — but the question is asked anyway because a future task that
    /// answers `true` should be a deliberate addition to this list rather than an
    /// oversight in a call site.
    pub fn may_publish_as_author_text(self) -> bool {
        false
    }
}

/// A cost estimate, produced before any work runs.
///
/// §22.11's budget guardrail: a caller compares this against its remaining budget and
/// declines if the quote does not fit. `None` means "this provider does not price this
/// task" — which is a legitimate answer for a local Ollama instance, and must not be
/// read as zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostQuote {
    /// Provider-estimated cost in the operator's configured unit (credits, or whatever
    /// §22.11 settled on). Kept as a string-free `i64` of micros to avoid float drift
    /// in a value that gates spending.
    pub micros: i64,
    /// What the unit is, for display and for the operator's budget configuration.
    pub unit: String,
}

impl CostQuote {
    /// A quote for a task the provider does not price — a local model, typically.
    pub fn unpriced(unit: &str) -> Self {
        CostQuote {
            micros: 0,
            unit: unit.to_string(),
        }
    }

    /// Whether this quote can be trusted as a ceiling for budgeting.
    ///
    /// A `None` here must stop the call, not be treated as free: §22.11 is a guardrail
    /// against unbounded spend, and "I could not price it" is not permission to guess.
    pub fn is_priced(&self) -> bool {
        !self.unit.is_empty()
    }
}

/// The verdict a provider returns for one dimension of one work.
///
/// A **structured** result, never prose: the model is asked for JSON against a schema and
/// anything unparseable or out of range is rejected. The reason is that a prose answer
/// cannot be validated, so it either gets trusted unvalidated or gets discarded at the
/// UI — and a number that *looks* like a measurement but is not one is worse than no
/// number at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreReadVerdict {
    /// Which dimension this is about (length, tone, tags-match…). Operator-configured,
    /// so this is an opaque string rather than an enum: the operator adds dimensions
    /// without a migration.
    pub dimension: String,
    /// 0.0–1.0. Constructed only through [`PreReadVerdict::new`], which rejects values
    /// outside the range, so no code path can hold a verdict with `score: 1.7`.
    pub score: f64,
    /// One sentence of author-facing rationale. Never shown to readers (§32.6).
    pub note: String,
    /// The task that produced this. §23.7: generated output is distinguished from
    /// author text, and a verdict with no task recorded cannot be shown at all.
    pub task: AiTask,
}

impl PreReadVerdict {
    /// Build a verdict, validating the score.
    ///
    /// The range check is the whole reason this constructor exists. §20.10.4 uses
    /// 0.0–1.0 for its quality score, and a provider asked for a number will cheerfully
    /// return 1.7 or −0.2. Clamping would silently launder a broken provider into a
    /// plausible number; rejecting records that the provider misbehaved.
    pub fn new(dimension: &str, score: f64, note: &str) -> Result<Self> {
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            // `AppError::internal` takes (context, source), so the message and the
            // underlying value are both named -- the score is the thing worth having in
            // the log when a provider is misbehaving repeatedly.
            return Err(AppError::internal(
                "AI verdict score outside 0.0-=1.0",
                anyhow::anyhow!("dimension {dimension:?} scored {score}"),
            ));
        }
        Ok(PreReadVerdict {
            dimension: dimension.to_string(),
            score,
            note: note.to_string(),
            task: AiTask::PreReadScoring,
        })
    }
}

/// Why a provider declined to answer.
///
/// A distinct type from an error because §35.5's precedent is explicit: "deterministic
/// abstention when no provider" is a **normal outcome**, not a failure. Collapsing the
/// two is what makes callers write `unwrap_or_default()` on an AI result and then treat
/// the default as a real answer.
/// Serde so a persisted report can record *why* a dimension is missing: "the provider was
/// not configured" and "the provider returned something unparseable" are different facts
/// to debug a week later, and collapsing them into a boolean loses the distinction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum AiAbstain {
    /// No provider is configured for this task (§23.7: disabled without configuration).
    NotConfigured,
    /// The reader or author has not consented to this provider (§23.7: opt out of
    /// specific providers).
    NoConsent,
    /// A quote was requested and none is available, so the call must not proceed
    /// (§22.11).
    Unpriced,
    /// The provider was called and its output did not validate. Recorded rather than
    /// retried: an endlessly retrying background job is how a bad provider configuration
    /// becomes an outage.
    InvalidOutput(String),
}

impl AiAbstain {
    /// Whether retrying could plausibly succeed.
    ///
    /// Only [`AiAbstain::InvalidOutput`] — a transient provider error is retried by the
    /// caller with backoff, but a consent decision or a missing configuration will still
    /// be the same answer in an hour.
    pub fn is_retryable(&self) -> bool {
        matches!(self, AiAbstain::InvalidOutput(_))
    }
}

/// The result of one AI call: a value, or a documented reason there isn't one.
pub type AiOutcome<T> = std::result::Result<T, AiAbstain>;

/// What a provider is asked to do.
///
/// The prompt and the private text travel together because §23.7 requires the provider's
/// retention and data-use disclosure to be shown *for this call*: a provider that is
/// fine with a summary may not be fine with a full chapter.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRequest {
    pub task: AiTask,
    /// The text to operate on. Private-text consent is checked against
    /// [`AiTask::touches_private_text`] before this is ever constructed.
    pub input: String,
    /// The question to answer, for the tasks that take one.
    pub instruction: String,
    /// The dimensions the operator configured, for [`AiTask::PreReadScoring`].
    pub dimensions: Vec<String>,
}

/// A provider that can answer [`AiRequest`]s.
///
/// **Boxed futures rather than `async fn` in trait.** A native `async fn` in a trait
/// loses the ability to state `Send` on the returned future, so an implementation holding
/// a `reqwest::Client` could not be awaited inside a spawned task — which is where every
/// caller of this trait belongs, since a worker is what runs these calls. Returning
/// `Pin<Box<dyn Future + Send>>` puts the bound back. The cost is one allocation per
/// call and four `Box::pin`s per adapter, which is nothing next to a network round trip.
pub trait AiProvider: Send + Sync {
    /// A stable name for `/api/v1/meta` and the consent UI. §23.7 requires
    /// "provider-specific retention and data-use disclosure", which needs an identity.
    fn name(&self) -> &str;

    /// Whether this provider can serve this task at all.
    ///
    /// §23.7: "disabled without configuration". Returning `false` is how an
    /// unconfigured deployment stays disabled — there is no separate "enabled" flag that
    /// can disagree with the provider actually working.
    fn available<'a>(&'a self, task: AiTask) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

    /// What this task would cost, before doing it.
    fn quote<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> Pin<Box<dyn Future<Output = AiOutcome<CostQuote>> + Send + 'a>>;

    /// Do the work.
    ///
    /// Implementations **must** validate provider output against the schema before
    /// returning it: an unvalidated string is not a verdict, and returning one here
    /// defeats [`PreReadVerdict::new`]. Adapters are the only place raw model output
    /// exists.
    fn run<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> Pin<Box<dyn Future<Output = AiOutcome<Vec<PreReadVerdict>>> + Send + 'a>>;

    /// What actually was charged, after the fact.
    ///
    /// Separate from [`AiProvider::quote`] because a provider may exceed its own estimate;
    /// a caller that reconciles budgets needs both numbers to notice when it did.
    fn charged<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> Pin<Box<dyn Future<Output = AiOutcome<CostQuote>> + Send + 'a>>;
}

/// Consent for one (author, provider, task) triple.
///
/// §23.7: "users may opt out of specific providers" and "explicit private-text consent".
/// Both are properties of the *author* at the moment of the call, because consent can be
/// withdrawn — which is why this is checked per call rather than cached as configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConsent {
    /// Whether the author has agreed to this provider at all.
    pub provider_allowed: bool,
    /// Whether they have agreed to private text going to it. Only meaningful for
    /// [`AiTask::touches_private_text`] tasks, and only ever consulted for those.
    pub private_text_allowed: bool,
}

impl AiConsent {
    /// Consent for a task that never sees private text.
    pub fn public_only(provider_allowed: bool) -> Self {
        AiConsent {
            provider_allowed,
            private_text_allowed: false,
        }
    }

    /// Consent for a task that does, which requires both.
    ///
    /// The asymmetry is deliberate: for a private-text task, `provider_allowed` alone is
    /// not enough. A reader who allowed *some* provider to summarise public posts has not
    /// thereby agreed to send their draft chapter to it.
    pub fn private_text(provider_allowed: bool, private_text_allowed: bool) -> Self {
        AiConsent {
            provider_allowed,
            private_text_allowed,
        }
    }

    /// Whether this call may proceed. The single gate every caller goes through.
    pub fn permits(&self, task: AiTask) -> AiOutcome<()> {
        if !self.provider_allowed {
            return Err(AiAbstain::NoConsent);
        }
        if task.touches_private_text() && !self.private_text_allowed {
            return Err(AiAbstain::NoConsent);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_outside_zero_to_one_is_rejected_not_clamped() {
        // Clamping would launder a misbehaving provider into a plausible-looking number,
        // and §20.10.4's downstream arithmetic would never know.
        assert!(PreReadVerdict::new("length", 1.7, "").is_err());
        assert!(PreReadVerdict::new("length", -0.2, "").is_err());
        assert!(PreReadVerdict::new("length", f64::NAN, "").is_err());
        assert!(PreReadVerdict::new("length", f64::INFINITY, "").is_err());
        assert!(PreReadVerdict::new("length", 0.0, "").is_ok());
        assert!(PreReadVerdict::new("length", 1.0, "").is_ok());
    }

    #[test]
    fn an_unavailable_provider_abstains_rather_than_erroring() {
        // §35.5's "deterministic abstention when no provider" — a distinct outcome, so
        // callers cannot `unwrap_or_default()` their way to treating it as a measurement.
        let abstain = AiAbstain::NotConfigured;
        assert!(!abstain.is_retryable());
        let outcome: AiOutcome<()> = Err(abstain);
        assert!(matches!(outcome, Err(AiAbstain::NotConfigured)));
    }

    #[test]
    fn only_invalid_output_is_retryable() {
        // Retrying a consent decision or a missing configuration retries the same answer
        // forever, which is how a bad configuration becomes an outage.
        assert!(AiAbstain::InvalidOutput("json".into()).is_retryable());
        for abstain in [
            AiAbstain::NotConfigured,
            AiAbstain::NoConsent,
            AiAbstain::Unpriced,
        ] {
            assert!(!abstain.is_retryable(), "{abstain:?}");
        }
    }

    #[test]
    fn a_private_text_task_needs_both_consents() {
        let task = AiTask::PreReadScoring;
        assert!(task.touches_private_text());
        // Allowing a provider is not thereby allowing private text to it.
        assert!(AiConsent::private_text(true, false).permits(task).is_err());
        assert!(AiConsent::private_text(false, true).permits(task).is_err());
        assert!(AiConsent::private_text(true, true).permits(task).is_ok());
    }

    #[test]
    fn a_public_task_ignores_the_private_text_consent() {
        // The opposite asymmetry: translation of a *published* chapter needs the provider
        // allowed, and nothing else.
        assert!(!AiTask::Translation.touches_private_text());
        assert!(AiConsent::public_only(true)
            .permits(AiTask::Translation)
            .is_ok());
        assert!(AiConsent::public_only(false)
            .permits(AiTask::Translation)
            .is_err());
    }

    #[test]
    fn no_task_may_publish_as_author_text() {
        // §23.7: "no automatic publication, generated output distinguished from author
        // text". Asserted across every variant so adding one that answers `true` has to
        // change this test on purpose.
        for task in [
            AiTask::Translation,
            AiTask::Summarization,
            AiTask::GrammarAssistance,
            AiTask::PromptAssistance,
            AiTask::Embeddings,
            AiTask::MetadataSuggestions,
            AiTask::CommentClassification,
            AiTask::SearchAssistance,
            AiTask::PreReadScoring,
        ] {
            assert!(!task.may_publish_as_author_text(), "{task:?}");
        }
    }

    #[test]
    fn an_unpriced_quote_is_not_a_free_quote() {
        // A local Ollama instance legitimately cannot price a task, but §22.11's guardrail
        // needs to distinguish "costs nothing" from "unknown".
        let quote = CostQuote::unpriced("");
        assert!(!quote.is_priced());
        assert!(CostQuote {
            micros: 0,
            unit: "credits".into()
        }
        .is_priced());
    }
}
