//! §23.7's OpenAI-compatible adapter.
//!
//! Step 3 of gap C. `crates/domain/src/ai.rs` defines the interface and
//! `crates/domain/src/preread.rs` the report shape; this is the first thing that actually
//! speaks to a provider.
//!
//! **What "OpenAI-compatible" means here is deliberately narrow.** The provider only needs
//! four capabilities — is it reachable, what does a task cost, run a prompt, what was it
//! actually charged — and all four are one `POST /v1/chat/completions` plus one `GET
//! /v1/models`. Anything more specific is a different provider. That narrowness is why a
//! stub server in the test file is sufficient verification: there is no streaming, no tool
//! calling, and no batch API to mock.
//!
//! **The three things this adapter must never do**, each of which is a §23.7 requirement
//! rather than a style choice:
//!
//! 1. **Return unvalidated model output.** A chat completion is a string a model wrote.
//!    It is parsed against a schema and anything that does not conform becomes
//!    [`AiAbstain::InvalidOutput`] and no verdict. Handing a caller a number that came
//!    from prose is how a plausible-looking score enters the product.
//! 2. **Send private text without a consent check.** The caller gates on
//!    [`AiConsent::permits`] before constructing an [`AiRequest`], but the adapter is the
//!    last place a missing check could be caught, so the task's own
//!    [`AiTask::touches_private_text`] is asserted here too. Defence in depth is not
//!    paranoia when the thing being protected is a reader's unpublished draft.
//! 3. **Treat "the provider is down" as a zero.** A network error is
//!    [`AiAbstain::NotConfigured`], not an empty verdict list, because an empty list and a
//!    provider outage look identical downstream and must not.

use lorehaven_domain::ai::{
    AiAbstain, AiOutcome, AiProvider, AiRequest, AiTask, CostQuote, PreReadVerdict,
};
use serde::Deserialize;

use crate::ai::http::AiHttp;

/// A client for any provider speaking the OpenAI chat-completions API.
pub struct OpenAiCompatible {
    http: AiHttp,
    model: String,
    /// §23.7: "provider-specific retention and data-use disclosure". Surfaced to the
    /// operator so the consent UI can say something true.
    retention_note: String,
}

/// The reply shape, minus everything unused.
///
/// `serde`'s default is to *ignore* unknown fields, which is what makes this compatible
/// with providers that add fields — and also what makes a typo'd field name silently
/// become `None`. The fields the adapter actually needs are therefore all read once, in
/// [`Self::verdicts`], and a missing one is an abstention rather than a default.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Option<Vec<Choice>>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    total_tokens: Option<u64>,
}

/// One verdict as the model was asked to return it.
///
/// Every field is a string in the prompt and a number here, and the conversion is what the
/// adapter exists to police: `"0.85"` parses, `"very high"` does not, and the second is an
/// abstention rather than a coerced `0.5`.
#[derive(Debug, Deserialize)]
struct RawVerdict {
    dimension: Option<String>,
    score: Option<f64>,
    note: Option<String>,
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, model: &str, retention_note: &str) -> Self {
        Self::with_http(AiHttp::new(base_url), model, retention_note)
    }

    /// Build against a caller-supplied [`AiHttp`].
    ///
    /// Not a test affordance: a caller that needs a proxy, a longer timeout, a shared
    /// connection pool, or — as here — private-text consent already has an `AiHttp`, and
    /// the alternative would be for this constructor to re-derive those settings and
    /// silently ignore them.
    pub fn with_http(http: AiHttp, model: &str, retention_note: &str) -> Self {
        OpenAiCompatible {
            http,
            model: model.to_string(),
            retention_note: retention_note.to_string(),
        }
    }

    pub fn retention_note(&self) -> &str {
        &self.retention_note
    }

    /// The prompt. A *prompt*, not a completion, because §23.7 requires the operator to
    /// be able to see what leaves the instance.
    fn prompt(request: &AiRequest) -> String {
        let dimensions = request.dimensions.join(", ");
        format!(
            "You are assessing a piece of fiction against author-configured dimensions.\n\
             Dimensions: {dimensions}\n\
             Return ONLY a JSON array. One object per dimension, with keys\n\
             \"dimension\" (string), \"score\" (number between 0 and 1), and \"note\"\n\
             (one short sentence). No prose outside the array.\n\n\
             Instruction: {}\n\n\
             Text:\n{}",
            request.instruction, request.input
        )
    }

    /// Parse and validate the model's reply.
    ///
    /// Four things are refused, and the refusal is always an abstention rather than a
    /// default value:
    ///
    /// * no choices, or an empty message — the model returned nothing usable;
    /// * `content` that is not a JSON array of verdicts — free prose, which is what a
    ///   model produces when it is asked a question it would rather answer in English;
    /// * a verdict missing `dimension` — an unattributable score cannot be reported, and
    ///   attributing it to a dimension the operator never configured is how a
    ///   prompt-injection result gets presented as a measurement;
    /// * a score outside 0..1 — [`PreReadVerdict::new`] does the check, so the clamp
    ///   decision is not duplicated here.
    fn verdicts(content: &str) -> AiOutcome<Vec<PreReadVerdict>> {
        let trimmed = content.trim();
        // Tolerate a fenced block. Models wrap JSON in ```json constantly, and rejecting
        // that would make the adapter fail on correct output.
        let body = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .map(|rest| rest.trim_end_matches('`').trim())
            .unwrap_or(trimmed);
        let raw: Vec<RawVerdict> = serde_json::from_str(body).map_err(|e| {
            AiAbstain::InvalidOutput(format!("response was not a JSON verdict array: {e}"))
        })?;
        if raw.is_empty() {
            return Err(AiAbstain::InvalidOutput(
                "response was an empty array".to_string(),
            ));
        }
        let mut out = Vec::with_capacity(raw.len());
        for verdict in raw {
            let dimension = verdict
                .dimension
                .ok_or_else(|| AiAbstain::InvalidOutput("verdict has no dimension".to_string()))?;
            let score = verdict.score.ok_or_else(|| {
                AiAbstain::InvalidOutput(format!("verdict for {dimension:?} has no score"))
            })?;
            // `PreReadVerdict::new` rejects an out-of-range score. Its error is an
            // AppError, so the refusal is translated here into the abstain the caller
            // understands -- the point is that no caller ever receives an unvalidated
            // number, not which error type says so.
            let checked =
                PreReadVerdict::new(&dimension, score, verdict.note.as_deref().unwrap_or(""))
                    .map_err(|e| AiAbstain::InvalidOutput(format!("{e}")))?;
            out.push(checked);
        }
        Ok(out)
    }

    async fn chat(&self, request: &AiRequest) -> AiOutcome<ChatResponse> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": "Reply with JSON only." },
                { "role": "user", "content": Self::prompt(request) },
            ],
            // A score per dimension. A hard cap is not optional: an unbounded completion
            // is an unbounded bill, and §22.11's guardrail is the caller's, so the
            // adapter has to be the one that bounds it.
            "max_tokens": 800,
            "temperature": 0,
        });
        self.http.post_json("/v1/chat/completions", &body).await
    }
}

impl AiProvider for OpenAiCompatible {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    fn available<'a>(
        &'a self,
        _task: AiTask,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { self.http.reachable().await })
    }

    fn quote<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AiOutcome<CostQuote>> + Send + 'a>>
    {
        Box::pin(async move {
            // §23.7 requires quoted costs. This adapter does not know the operator's
            // price per token, so the honest quote is "unpriced" rather than a guess --
            // and `CostQuote::is_priced()` is what stops a caller reading it as free.
            let _ = request;
            Ok(CostQuote::unpriced("tokens"))
        })
    }

    fn run<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = AiOutcome<Vec<PreReadVerdict>>> + Send + 'a>,
    > {
        Box::pin(async move {
            // Defence in depth on consent. The caller gates this, but the adapter is the
            // last place a forgotten check could be caught, and the thing being protected
            // is somebody's unpublished draft.
            if request.task.touches_private_text() && !self.http.private_text_allowed {
                return Err(AiAbstain::NoConsent);
            }
            let response = self.chat(request).await?;
            let content = response
                .choices
                .and_then(|mut c| c.pop())
                .and_then(|c| c.message)
                .and_then(|m| m.content)
                .ok_or_else(|| {
                    AiAbstain::InvalidOutput("no message content in the reply".to_string())
                })?;
            Self::verdicts(&content)
        })
    }

    fn charged<'a>(
        &'a self,
        request: &'a AiRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AiOutcome<CostQuote>> + Send + 'a>>
    {
        Box::pin(async move {
            let response = self.chat(request).await?;
            // Reported from the provider's own `usage` when it sends one. Absent usage is
            // "unpriced", not zero, for the same reason `quote` is.
            Ok(match response.usage.and_then(|u| u.total_tokens) {
                // Tokens are not credits; the operator converts. The factor here is a
                // unit conversion (milli-tokens), NOT a price: nothing in the code knows
                // what a token costs, because §23.7 gives that to the administrator. So
                // this is `priced: false` -- a count, not a quote -- and a caller gating on
                // §22.11's budget gets the real number from its own price table.
                //
                // Reporting it as priced would be the subtle bug here: it would let a
                // budget check pass on a figure that is not money.
                Some(tokens) => CostQuote {
                    micros: (tokens as i64).saturating_mul(1_000),
                    unit: "milli-tokens".to_string(),
                    priced: false,
                },
                None => CostQuote::unpriced("tokens"),
            })
        })
    }
}
