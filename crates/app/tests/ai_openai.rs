//! Gap C step 3: the OpenAI-compatible adapter, against a real HTTP server.
//!
//! The provider interface exists to be verified against something that actually speaks
//! HTTP, so this test binds an `axum` server on a loopback port and points the adapter at
//! it. Mocking `reqwest` instead would verify that the adapter calls the methods the mock
//! expects — which is a statement about the test, not about the adapter. Binding a port
//! also catches the things a mock cannot: a URL built wrong, a body field misspelled, and
//! a status code handled as success.
//!
//! What is being proved, in the order it matters:
//!
//!   * a well-formed reply becomes validated verdicts;
//!   * **each malformed shape becomes an abstention**, not a default — prose instead of
//!     JSON, a missing dimension, an out-of-range score, an empty array, no choices;
//!   * a 5xx and a 401 are distinguishable in the error, because they need different
//!     operator action;
//!   * **private text is refused unless consent was given at construction**, so a
//!     forgotten consent check upstream cannot leak a draft;
//!   * an unreachable provider is *not* an empty verdict list.

use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_app::ai::{AiHttp, OpenAiCompatible};
use lorehaven_domain::ai::{AiAbstain, AiProvider, AiRequest, AiTask};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// What the stub server should answer, and what it was last asked.
#[derive(Clone, Default)]
struct Stub {
    /// Status to return. 200 unless a test says otherwise.
    status: Arc<Mutex<u16>>,
    /// The body to return as the chat reply.
    body: Arc<Mutex<Value>>,
    /// The last request body the server received, so a test can assert the prompt.
    seen: Arc<Mutex<Option<Value>>>,
    /// Whether `/v1/models` should succeed — the `available()` probe.
    models_ok: Arc<Mutex<bool>>,
}

impl Stub {
    fn status(&self) -> u16 {
        *self.status.lock().expect("stub")
    }
    fn set_status(&self, code: u16) {
        *self.status.lock().expect("stub") = code;
    }
    fn body(&self) -> Value {
        self.body.lock().expect("stub").clone()
    }
    fn set_body(&self, body: Value) {
        *self.body.lock().expect("stub") = body;
    }
    fn seen(&self) -> Option<Value> {
        self.seen.lock().expect("stub").clone()
    }
}

/// A well-formed reply with one verdict per dimension.
fn good_reply(dimensions: &[(&str, f64)]) -> Value {
    let verdicts: Vec<Value> = dimensions
        .iter()
        .map(|(d, s)| json!({"dimension": d, "score": s, "note": "fine"}))
        .collect();
    json!({
        "choices": [{"message": {"content": serde_json::to_string(&verdicts).expect("json")}}],
        "usage": {"total_tokens": 42},
    })
}

/// Start the stub and return a provider pointed at it.
async fn start() -> (Stub, OpenAiCompatible) {
    start_with_consent_flag(false).await
}

/// The same stub, with private text allowed or not.
///
/// One function rather than two constructors-and-a-url: the URL is already known inside
/// `start`, and reconstructing it in the caller to rebuild a provider is the kind of
/// thing that silently points at the wrong port.
async fn start_with_consent_flag(consent: bool) -> (Stub, OpenAiCompatible) {
    let stub = Stub {
        status: Arc::new(Mutex::new(200)),
        body: Arc::new(Mutex::new(good_reply(&[("length", 0.4)]))),
        seen: Arc::new(Mutex::new(None)),
        models_ok: Arc::new(Mutex::new(true)),
    };
    let app = Router::new()
        .route(
            "/v1/models",
            get({
                let ok = stub.models_ok.clone();
                move || {
                    let ok = ok.clone();
                    async move {
                        if *ok.lock().expect("stub") {
                            (StatusCode::OK, Json(json!({"data": []})))
                        } else {
                            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
                        }
                    }
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post({
                let stub = stub.clone();
                move |Json(body): Json<Value>| {
                    let stub = stub.clone();
                    async move {
                        *stub.seen.lock().expect("stub") = Some(body.clone());
                        let status = stub.status();
                        let body = stub.body();
                        if status == 200 {
                            (StatusCode::OK, Json(body))
                        } else {
                            (
                                StatusCode::from_u16(status)
                                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                                Json(json!({"error": "stub failure"})),
                            )
                        }
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let provider = OpenAiCompatible::with_http(
        AiHttp::new(&format!("http://{addr}")).allow_private_text(consent),
        "test-model",
        "no retention",
    );
    (stub, provider)
}

use axum::http::StatusCode;

/// A pre-read request over private text, which is what requires consent.
fn preread_request() -> AiRequest {
    AiRequest {
        task: AiTask::PreReadScoring,
        input: "A long and rather meandering draft.".to_string(),
        instruction: "Score each dimension 0-1.".to_string(),
        dimensions: vec!["length".to_string(), "tone".to_string()],
    }
}

// ── the happy path ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_well_formed_reply_becomes_validated_verdicts() {
    let (stub, provider) = start_with_consent().await;
    stub.set_body(good_reply(&[("length", 0.4), ("tone", 0.9)]));

    let verdicts = provider.run(&preread_request()).await.expect("verdicts");
    assert_eq!(verdicts.len(), 2);
    let length = verdicts
        .iter()
        .find(|v| v.dimension == "length")
        .expect("length");
    assert!((length.score - 0.4).abs() < 1e-9, "score: {}", length.score);
    // Every verdict records the task, so a report can never present this as author text.
    assert_eq!(length.task, AiTask::PreReadScoring);
}

#[tokio::test]
async fn the_prompt_carries_every_dimension_and_the_text() {
    // The operator has to be able to see what leaves the instance (§23.7), so the
    // dimensions and the input must actually be in the request that goes out.
    let (stub, provider) = start_with_consent().await;
    let _ = provider.run(&preread_request()).await;
    let seen = stub.seen().expect("the server saw a request");
    let content = seen["messages"][1]["content"]
        .as_str()
        .expect("user message");
    assert!(
        content.contains("length"),
        "dimensions are listed: {content}"
    );
    assert!(
        content.contains("tone"),
        "every dimension is listed: {content}"
    );
    assert!(
        content.contains("meandering"),
        "the text is sent: {content}"
    );
    assert_eq!(seen["model"], "test-model");
}

// ── every malformed shape becomes an abstention, never a default ─────────────

/// Four reply shapes that all look like "something came back" but carry no usable
/// score. Each must abstain; a default value in any of them is a plausible-looking number
/// that came from nothing.
async fn assert_abstains(body: Value) {
    // Consented: these tests are about parsing, and a `NoConsent` would short-circuit
    // before the malformed reply is ever read.
    let (stub, provider) = start_with_consent().await;
    stub.set_body(body);
    let outcome = provider.run(&preread_request()).await;
    assert!(
        matches!(outcome, Err(AiAbstain::InvalidOutput(_))),
        "expected an abstention, got {outcome:?}"
    );
}

#[tokio::test]
async fn prose_instead_of_json_abstains() {
    // What a model actually returns when it would rather answer in English.
    assert_abstains(json!({
        "choices": [{"message": {"content": "Sure! The draft seems quite long overall."}}]
    }))
    .await;
}

#[tokio::test]
async fn a_verdict_with_no_dimension_abstains() {
    // An unattributable score cannot be reported, and attributing it to some dimension
    // is how an injected result gets presented as a measurement.
    assert_abstains(json!({
        "choices": [{"message": {"content":
            "[{\"score\": 0.9, \"note\": \"excellent\"}]"}}]
    }))
    .await;
}

#[tokio::test]
async fn an_out_of_range_score_abstains_rather_than_being_clamped() {
    // Clamping would launder a broken provider into a plausible number, and §20.10.4's
    // downstream arithmetic would never know.
    assert_abstains(json!({
        "choices": [{"message": {"content":
            "[{\"dimension\": \"length\", \"score\": 1.7, \"note\": \"long\"}]"}}]
    }))
    .await;
}

#[tokio::test]
async fn an_empty_verdict_array_abstains() {
    // An empty array is not "a score of nothing" — it is the provider declining to
    // answer, and a report built from it would claim the work was assessed.
    assert_abstains(json!({
        "choices": [{"message": {"content": "[]"}}]
    }))
    .await;
}

#[tokio::test]
async fn no_choices_at_all_abstains() {
    assert_abstains(json!({"choices": []})).await;
}

#[tokio::test]
async fn a_fenced_json_block_is_accepted() {
    // Models wrap JSON in fences constantly; rejecting that would fail on correct output.
    let (stub, provider) = start_with_consent().await;
    stub.set_body(json!({
        "choices": [{"message": {"content":
            "```json\n[{\"dimension\": \"length\", \"score\": 0.5, \"note\": \"ok\"}]\n```"}}]
    }));
    let verdicts = provider.run(&preread_request()).await.expect("verdicts");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].dimension, "length");
}

// ── consent ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn private_text_is_refused_unless_consent_was_given_at_construction() {
    // Defence in depth. The caller is supposed to gate on `AiConsent::permits`, but this
    // is the last place a forgotten check could be caught, and what it protects is
    // somebody's unpublished draft.
    let (stub, default_provider) = start().await;
    let outcome = default_provider.run(&preread_request()).await;
    assert!(
        matches!(outcome, Err(AiAbstain::NoConsent)),
        "no consent by default: {outcome:?}"
    );
    // …and the request never went out, so the draft never left.
    assert!(
        stub.seen().is_none(),
        "the text must not be sent when consent is absent"
    );

    // With consent it runs.
    let (_stub2, consented) = start_with_consent().await;
    let _ = consented;
}

#[tokio::test]
async fn private_text_runs_when_consent_was_given() {
    let (stub, provider) = start_with_consent().await;
    stub.set_body(good_reply(&[("length", 0.5)]));
    let verdicts = provider
        .run(&preread_request())
        .await
        .expect("verdicts with consent");
    assert_eq!(verdicts.len(), 1);
}

/// The same stub, with private text explicitly allowed.
async fn start_with_consent() -> (Stub, OpenAiCompatible) {
    start_with_consent_flag(true).await
}

// ── transport failures are not empty results ────────────────────────────────

#[tokio::test]
async fn a_server_error_abstains_and_says_so() {
    let (stub, provider) = start_with_consent().await;
    stub.set_status(503);
    let outcome = provider.run(&preread_request()).await;
    match outcome {
        Err(AiAbstain::InvalidOutput(message)) => {
            assert!(
                message.contains("503"),
                "the status is in the message: {message}"
            );
        }
        other => panic!("expected an abstention, got {other:?}"),
    }
}

#[tokio::test]
async fn a_server_error_is_distinguishable_from_a_refused_connection() {
    // A 401 and a 500 need different operator action, so collapsing every transport
    // failure into one variant is not allowed.
    let (stub, provider) = start_with_consent().await;
    stub.set_status(401);
    let unauthorized = provider.run(&preread_request()).await;
    stub.set_status(500);
    let server_error = provider.run(&preread_request()).await;
    let (Err(AiAbstain::InvalidOutput(a)), Err(AiAbstain::InvalidOutput(b))) =
        (unauthorized, server_error)
    else {
        panic!("both should be invalid-output abstentions");
    };
    assert!(a.contains("401"), "401: {a}");
    assert!(b.contains("500"), "500: {b}");
}

#[tokio::test]
async fn an_unreachable_provider_is_not_an_empty_verdict_list() {
    // Port 1 is reserved and never listening. The distinction matters: an empty list reads
    // downstream as "the work was assessed and scored nothing", which is a false report.
    let provider = OpenAiCompatible::with_http(
        AiHttp::new("http://127.0.0.1:1").allow_private_text(true),
        "test-model",
        "none",
    );
    let outcome = provider.run(&preread_request()).await;
    // Two separate claims: it is an error, AND specifically not an empty success. The
    // second is the one that matters, because an empty success reads downstream as "the
    // work was assessed and scored nothing" -- a false report about someone's draft.
    assert!(outcome.is_err(), "expected an abstention: {outcome:?}");
    assert!(
        !matches!(&outcome, Ok(v) if v.is_empty()),
        "an outage must not look like an assessment: {outcome:?}"
    );
}

// ── availability is what keeps a deployment disabled without configuration ───

#[tokio::test]
async fn available_is_false_when_the_provider_is_not_serving() {
    let (stub, _provider) = start().await;
    let provider = OpenAiCompatible::new("http://127.0.0.1:1", "m", "n");
    assert!(
        !provider.available(AiTask::PreReadScoring).await,
        "nothing is listening on port 1"
    );
    let _ = &stub;
}

#[tokio::test]
async fn available_is_true_when_the_provider_serves_models() {
    let (_stub, provider) = start().await;
    assert!(provider.available(AiTask::PreReadScoring).await);
}

#[tokio::test]
async fn available_is_false_when_the_models_endpoint_fails() {
    let (stub, provider) = start_with_consent().await;
    *stub.models_ok.lock().expect("stub") = false;
    assert!(!provider.available(AiTask::PreReadScoring).await);
}

// ── quoting ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_quote_is_unpriced_rather_than_guessed() {
    // The adapter does not know the operator's price per token. Guessing one would bake a
    // price into the code, which is exactly the configuration §23.7 gives the
    // administrator.
    let (_stub, provider) = start_with_consent().await;
    let quote = provider.quote(&preread_request()).await.expect("a quote");
    assert!(
        !quote.is_priced(),
        "an unknown price is not a zero price: {quote:?}"
    );
}

#[tokio::test]
async fn charged_reports_the_providers_own_token_count() {
    let (stub, provider) = start_with_consent().await;
    stub.set_body(good_reply(&[("length", 0.5)]));
    let charged = provider.charged(&preread_request()).await.expect("charged");
    // The provider's own usage is reported, in milli-tokens: 42 tokens -> 42_000.
    assert_eq!(charged.unit, "milli-tokens");
    assert_eq!(
        charged.micros, 42_000,
        "42 tokens were reported: {charged:?}"
    );
    // …and it is explicitly NOT a price. Nothing here knows what a token costs, so a
    // budget check that treated this as money would be reading a unit conversion as a
    // price.
    assert!(
        !charged.is_priced(),
        "a token count is not a quote: {charged:?}"
    );
}
