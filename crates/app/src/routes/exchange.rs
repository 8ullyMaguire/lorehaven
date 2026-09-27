//! M57 — the metadata exchange endpoint (spec §11.17, §15.17, §19.14).
//!
//! Three doors, and the ordering of the checks in each handler is the spec:
//!
//! ```text
//! GET  /exchange/version      → supported ExchangeVersion range
//! POST /exchange/signals      → submit a SignalBatch
//! GET  /exchange/canonical    → fetch canonical metadata
//! ```
//!
//! **The opt-in comes first, and it answers 404 rather than 403.** §11.17: "No
//! request shape turns on a server that did not enable it." A 403 would confirm
//! the door exists, tell a prober that this instance runs an exchange, and
//! invite exactly the retry loop a disabled endpoint should not attract. Every
//! handler checks `settings.enabled` before it checks anything else, including
//! before it looks at the session — so an unauthenticated caller learns nothing
//! about whether the exchange exists.
//!
//! **Trust is asymmetric and not configurable** (§19.14). TL1 submits, TL3
//! curates. The bars are constants in `lorehaven_domain::exchange`, read from
//! there rather than from config, because §0.3 makes trust non-purchasable and a
//! config key would be a purchased moderation authority wearing a different
//! name.
//!
//! **The response never carries a submitter or a holder count** (§11.17). Neither
//! is a field on the acknowledgement type, so there is nothing to strip at the
//! edge — the omission is structural.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use lorehaven_domain::exchange as exch;
use lorehaven_lore_metadata::{
    CanonicalBatch, CanonicalWork, EntityRef, ExchangeVersion, ReviewStatus, SignalBatch,
};

use crate::auth::MaybeSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/exchange/version", get(get_version))
        .route("/exchange/signals", post(submit_signals))
        .route("/exchange/canonical", get(get_canonical))
        // §15.17/§19.14: the review queue and the curation act. Both are
        // governance surfaces, gated on trust in the handler rather than on a
        // flag here — see `get_review_queue` and `curate_entity`.
        .route("/exchange/review-queue", get(get_review_queue))
        .route("/exchange/entities/curate", post(curate_entity))
}

/// The settings row, or 404 if the operator never enabled the exchange.
///
/// Split out because all three handlers need it first, and because the 404 is a
/// spec requirement rather than an error-handling convenience.
async fn require_enabled(state: &AppState) -> ApiResult<lorehaven_db::exchange::ExchangeSettings> {
    // `get_settings` returns an `anyhow::Result`, so no `.into()` here — unlike
    // `trust_for` below, which returns a `sqlx::Error` and needs one. The two
    // look alike at the call site and the difference is invisible until
    // clippy names the redundant conversion.
    let settings = lorehaven_db::exchange::get_settings(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    match settings {
        Some(s) if s.enabled => Ok(s),
        // §11.17: not 403. A refusal that names the resource tells a prober the
        // resource exists.
        _ => Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "exchange",
        })),
    }
}

/// The signed-in account, or a refusal. TL1 is the bar to submit (§19.14).
async fn require_submitter(
    state: &AppState,
    session: &crate::auth::SessionUser,
) -> ApiResult<String> {
    // `account_id` is an `AccountId`, and `trust_for` takes `&str`. The
    // conversion is explicit rather than a `Deref` away, because the typed id
    // existing is what stops an account id being passed where a pseud id belongs
    // — a mistake that compiles cleanly when both are `String`.
    let account_id = session.account_id.to_string();
    let trust = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if !exch::may_submit(trust) {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    Ok(account_id)
}

/// The supported version range. The one door that answers when the exchange is
/// off, because a client needs to know whether negotiation is even possible
/// before it tries — and §11.17's "no request shape turns on a server that did
/// not enable it" is about *data*, not about refusing a version probe that
/// returns no instance data at all.
///
/// Returning the range rather than a refusal is deliberate: a client that cannot
/// agree a version is told what is possible in the same response, instead of
/// needing a second round trip to discover that.
pub async fn get_version(
    State(state): State<AppState>,
    // Declared and unused. A handler holding `State<AppState>` with no audience
    // extractor is a data-leak vector the route-inventory test exists to catch,
    // and it is right to: an extractor that gates behaviour is the difference
    // between a public door and an open one. This door is genuinely anonymous —
    // it returns a version range and a boolean, no instance data — so the
    // extractor is present to *state* that, not because it needs to.
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    // `get_settings` returns an `anyhow::Result`, so no `.into()` here — unlike
    // `trust_for` below, which returns a `sqlx::Error` and needs one. The two
    // look alike at the call site and the difference is invisible until
    // clippy names the redundant conversion.
    let settings = lorehaven_db::exchange::get_settings(state.db())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let enabled = settings.map(|s| s.enabled).unwrap_or(false);
    let range = ExchangeVersion::supported_range();
    Ok(Json(json!({
        "current": ExchangeVersion::CURRENT.0,
        "supported": { "min": range.min.0, "max": range.max.0 },
        "enabled": enabled,
    })))
}

/// Submit a batch of signals. TL1 (§19.14).
pub async fn submit_signals(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    // The body is taken as raw bytes and deserialised here rather than through
    // axum's `Json` extractor. That is the whole deliverable of §11.17: a
    // payload carrying `reader_id` must be "rejected by name", and
    // `deny_unknown_fields` alone does not deliver that over HTTP.
    //
    // `Json<T>`'s rejection is a 422 with an **empty body** — axum discards the
    // serde error, and the field name goes with it. So a client sending
    // `reader_id` got a bare 422 and learned nothing: the request was refused,
    // but not *by name*, and a client that sent a field it believed was
    // accepted has no way to find out which one was dropped. The first version
    // of this handler did exactly that, and the test that caught it is
    // `every_prohibited_field_is_refused_rather_than_ignored`.
    //
    // Deserialising by hand routes the failure through `AppError`, which renders
    // the spec's own error envelope with `field_errors` — so the name arrives
    // where a client can read it.
    let batch: SignalBatch = serde_json::from_slice(&body).map_err(|e| {
        ApiError(lorehaven_domain::AppError::field(
            "body",
            format!("the signal batch is not valid: {e}"),
        ))
    })?;
    // 1. The opt-in, before the session and before validation.
    let settings = require_enabled(&state).await?;
    // 2. Version negotiation. Refused out of range, and the refusal carries the
    //    supported range so the client can adapt without a second call.
    if let Err(unsupported) = ExchangeVersion::negotiate(batch.version) {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "version",
            unsupported.to_string(),
        )));
    }
    // 3. A session, then the trust bar. The order matters for what an anonymous
    //    caller learns: with the exchange enabled they learn the door exists and
    //    needs an account, which is the same thing every other write door says.
    let session = session.ok_or_else(|| ApiError(lorehaven_domain::AppError::AuthRequired))?;
    let account_id = require_submitter(&state, &session).await?;
    // 4. The rate limit, per account over a trailing hour (§11.17). Checked
    //    before validation so a client spamming malformed batches is still
    //    charged for them — otherwise the limit is free to abuse by making every
    //    request invalid.
    let since = one_hour_ago();
    let recent = lorehaven_db::exchange::count_recent_submissions(state.db(), &account_id, &since)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if recent >= settings.rate_limit_per_hour {
        return Err(ApiError(lorehaven_domain::AppError::RateLimited {
            retry_after_secs: 3600,
        }));
    }
    // 5. Validation. Every refusal is named (§11.17: refused by name, not
    //    quietly trimmed).
    if let Err(rejection) = exch::validate_batch(&batch) {
        return Err(ApiError(match rejection {
            exch::BatchRejection::Empty => lorehaven_domain::AppError::field(
                "signals",
                "a batch must carry at least one signal",
            ),
            exch::BatchRejection::UnsupportedVersion(v) => {
                lorehaven_domain::AppError::field("version", v)
            }
            exch::BatchRejection::SignalWithoutTitle { index } => {
                lorehaven_domain::AppError::field(
                    &format!("signals[{index}].title"),
                    "a signal must carry a title",
                )
            }
            exch::BatchRejection::UnknownEntityKind { index, kind } => {
                lorehaven_domain::AppError::field(
                    &format!("signals[{index}].kind"),
                    format!("unknown entity kind `{kind}`"),
                )
            }
            exch::BatchRejection::TooManySignals { count, limit } => {
                lorehaven_domain::AppError::field(
                    "signals",
                    format!("a batch may carry at most {limit} signals, this one has {count}"),
                )
            }
        }));
    }

    // §16.16.1's dedup key is the *submitting* instance. For a signal arriving
    // over HTTP that is the instance the client authenticated as, which is this
    // instance's own configured id — a client does not get to nominate it, or
    // latent demand would collapse into one row per spoofed name.
    let source_instance = settings.instance_id.clone();
    let mut results = Vec::with_capacity(batch.signals.len());
    for signal in &batch.signals {
        let hash = exch::content_hash(signal);
        let payload = serde_json::to_string(signal).map_err(|e| {
            ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
                "a validated signal did not serialise: {e}"
            )))
        })?;
        let inserted = lorehaven_db::exchange::store_signal(
            state.db(),
            &hash,
            None,
            &account_id,
            source_instance.as_deref(),
            &payload,
        )
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
        if !inserted {
            // §11.17's deduplication. A re-import costs nothing and creates no
            // second record — and, importantly, does not reinforce the entity
            // counts either, because a duplicate signal is not new evidence.
            results.push(exch::SignalOutcome {
                content_hash: hash,
                entities: Vec::new(),
                duplicate: true,
            });
            continue;
        }
        let entities: Vec<(String, String, String)> = exch::named_entities(signal)
            .into_iter()
            .map(|(kind, norm, original)| (kind.as_str().to_string(), original, norm))
            .collect();
        lorehaven_db::exchange::reinforce_entities(state.db(), &hash, &entities)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
        // §15.17: "A new name is usable immediately, and visibly so." Usable
        // means the *instance's own taxonomy* — a node `search_nodes` finds, a
        // tag browser lists, and `tag_work` can attach. Recording the name only
        // in the exchange's own table would satisfy `GET /canonical` while
        // leaving the tag invisible everywhere else, which is exactly the stall
        // §15.17 says the unverified/curated split exists to avoid.
        //
        // So each entity is also ensured as a taxonomy node, unverified. The
        // node is the usable artefact; `canonical_entities` stays the exchange's
        // own provenance record of which signal named what.
        for (kind, value, norm) in &entities {
            let node = lorehaven_db::taxonomy::ensure_node_from_signal(state.db(), kind, value)
                .await
                .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            // §15.17: "A signal naming a known tag adds an alias rather than
            // creating a duplicate." When the submitted spelling differs from
            // the normalised form — mixed case, doubled spacing — that spelling
            // is the variant a curator needs before deciding on a merge, so it
            // is recorded against the node. Only when it differs: re-recording
            // the canonical spelling as its own alias is noise.
            if value.to_lowercase() != *norm {
                lorehaven_db::taxonomy::create_alias(state.db(), value, &node.id, "exchange")
                    .await
                    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
            }
        }
        results.push(exch::SignalOutcome {
            content_hash: hash,
            entities: entities
                .iter()
                .map(|(kind, _, norm)| EntityRef {
                    kind: kind.clone(),
                    id: norm.clone(),
                    // Aliases are what a curator merges into; an uncurated
                    // entity has none recorded on it yet, because the
                    // alternative — filling them in from the spellings seen —
                    // would publish a curator's unfinished work as canonical.
                    aliases: Vec::new(),
                })
                .collect(),
            duplicate: false,
        });
    }
    let ack = exch::SignalAck {
        accepted: results.len(),
        duplicates: results.iter().filter(|r| r.duplicate).count(),
        results,
    };
    Ok(Json(serde_json::to_value(ack).map_err(|e| {
        ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
            "the acknowledgement did not serialise: {e}"
        )))
    })?))
}

#[derive(Debug, Deserialize)]
pub struct CanonicalQuery {
    /// Comma-separated `kind:norm` pairs: `?entity=tag:slow+burn,character:x`.
    ///
    /// A single comma-separated string rather than a repeated query key. Axum's
    /// `Query` deserialises repeated keys into a `Vec` only with the optional
    /// `serde_html_form` feature, and the first version of this used
    /// `Vec<String>` — which compiled, and then answered **400 with an empty
    /// body** for every request, because the extractor rejected the query before
    /// the handler ran. That is the worst possible shape for a spec rule: the
    /// 404-when-disabled check never executed, so a disabled exchange reported
    /// 400 and the opt-in looked broken rather than closed.
    ///
    /// The empty default matters for the same reason: a request with no
    /// `entity` at all must reach the handler and get the opt-in's 404, not an
    /// extractor's 400.
    #[serde(default)]
    pub entity: String,
}

/// Fetch canonical metadata for named entities.
///
/// §11.17: the response is canonical metadata only, and it "never reveals who
/// submitted a signal, and it never reveals how many accounts hold a work".
pub async fn get_canonical(
    State(state): State<AppState>,
    Query(q): Query<CanonicalQuery>,
    // Declared for the same reason as `get_version`: this door is anonymous by
    // design, and an unauthenticated caller is allowed to read canonical
    // metadata — §11.17 gates it on the operator's opt-in, not on identity.
    MaybeSession(_session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let _settings = require_enabled(&state).await?;
    let mut wanted: Vec<(String, String)> = Vec::new();
    // Split on commas and drop blanks, so a trailing comma or `?entity=` is not
    // a 422 about an empty pair.
    for raw in q.entity.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((kind, norm)) = raw.split_once(':') else {
            return Err(ApiError(lorehaven_domain::AppError::field(
                "entity",
                format!("`{raw}` is not `kind:norm`"),
            )));
        };
        let Some(kind) = exch::EntityKind::parse(kind) else {
            return Err(ApiError(lorehaven_domain::AppError::field(
                "entity",
                format!("`{kind}` is not a known entity kind"),
            )));
        };
        wanted.push((kind.as_str().to_string(), exch::normalise(norm)));
    }
    let rows = lorehaven_db::exchange::get_entities(state.db(), &wanted)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let works: Vec<CanonicalWork> = rows
        .into_iter()
        .map(|r| CanonicalWork {
            entity: EntityRef {
                kind: r.kind,
                id: r.norm,
                aliases: Vec::new(),
            },
            title: r.canonical,
            // Everything below is what this instance has not corrected yet. An
            // empty vec and a null are the honest answer for an auto-created
            // entity; inventing a value here would be the instance asserting a
            // canonical fact it has no evidence for.
            author_names: Vec::new(),
            fandom: None,
            tags: Vec::new(),
            characters: Vec::new(),
            relationships: Vec::new(),
            content_rating: None,
            word_count: None,
            completion: None,
            // `i64` in the database, `u64` on the wire. A negative count is
            // impossible (the column is only ever incremented), so the cast is
            // total rather than a clamp-and-hope.
            signal_count: r.signal_count.max(0) as u64,
            curated_at: r.curated_at,
            // Stored as `curated`, matching `taxonomy_nodes`; the wire type
            // calls the same state `Verified`. One vocabulary in storage, the
            // protocol's own on the wire — and a third spelling here would be a
            // third way for the two tables to disagree.
            review_status: if r.review_status == "curated" {
                ReviewStatus::Verified
            } else {
                ReviewStatus::Unverified
            },
        })
        .collect();
    let batch = CanonicalBatch::new(works);
    Ok(Json(serde_json::to_value(batch).map_err(|e| {
        ApiError(lorehaven_domain::AppError::Internal(anyhow::anyhow!(
            "the canonical batch did not serialise: {e}"
        )))
    })?))
}

/// The entities awaiting curation, most-reinforced first (§15.17's queue).
///
/// Operator-only in effect: the queue names what the instance has been told and
/// has not yet reviewed, which is a moderation workload rather than public
/// metadata. Gated on trust in the handler, not on a route flag, because §19.14
/// makes the bar a trust threshold and §0.3 makes trust non-purchasable.
pub async fn get_review_queue(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
) -> ApiResult<Json<Value>> {
    let _ = require_enabled(&state).await?;
    let session = session.ok_or(ApiError(lorehaven_domain::AppError::AuthRequired))?;
    let trust = lorehaven_db::governance::trust_for(state.db(), &session.account_id.to_string())
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    if !exch::may_curate(trust) {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    let nodes = lorehaven_db::taxonomy::list_unverified_nodes(state.db(), 100)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    Ok(Json(json!({
        "entities": nodes
            .iter()
            .map(|n| json!({
                "kind": n.kind,
                "canonical": n.canonical,
                "norm": n.norm,
                "review_status": n.review_status,
                // Review priority. Not a demand weight, not a reader count.
                "signal_count": n.signal_count,
            }))
            .collect::<Vec<_>>(),
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct CurateBody {
    pub kind: String,
    /// The normalised name, as the queue reports it.
    pub norm: String,
    /// The canonical form the curator is setting.
    pub canonical: String,
}

/// Curate an entity: set its canonical form and mark it verified (§19.4 quorum
/// work, gated to TL3).
///
/// §15.17 requires the originating signals to be **retained as provenance,
/// never rewritten**, so this updates the node and the exchange's entity record
/// and touches nothing in `exchange_signals` or `exchange_signal_entities`. That
/// is why a curator cannot "tidy" a signal by editing it: the function has no
/// path that writes those tables.
pub async fn curate_entity(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    axum::Json(body): axum::Json<CurateBody>,
) -> ApiResult<Json<Value>> {
    let _ = require_enabled(&state).await?;
    let session = session.ok_or(ApiError(lorehaven_domain::AppError::AuthRequired))?;
    let account_id = session.account_id.to_string();
    let trust = lorehaven_db::governance::trust_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e.into())))?;
    // §19.14: TL3. A change to a canonical value other readers will receive is a
    // governance act, held to the governance bar.
    if !exch::may_curate(trust) {
        return Err(ApiError(lorehaven_domain::AppError::AccessDenied));
    }
    if exch::EntityKind::parse(&body.kind).is_none() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "kind",
            format!("`{}` is not a known entity kind", body.kind),
        )));
    }
    let norm = exch::normalise(&body.norm);
    let canonical = body.canonical.trim();
    if canonical.is_empty() {
        return Err(ApiError(lorehaven_domain::AppError::field(
            "canonical",
            "a curated entity needs a canonical form to display",
        )));
    }
    let node_curated =
        lorehaven_db::taxonomy::curate_node(state.db(), &body.kind, &norm, canonical)
            .await
            .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    let entity_curated = lorehaven_db::exchange::curate_entity(
        state.db(),
        &body.kind,
        &norm,
        canonical,
        &account_id,
    )
    .await
    .map_err(|e| ApiError(lorehaven_domain::AppError::Internal(e)))?;
    if !node_curated && !entity_curated {
        return Err(ApiError(lorehaven_domain::AppError::NotFound {
            resource: "entity",
        }));
    }
    Ok(Json(
        json!({ "curated": true, "kind": body.kind, "norm": norm, "canonical": canonical }),
    ))
}

/// An RFC 3339 timestamp one hour ago, for the rate-limit window.
fn one_hour_ago() -> String {
    let now = time::OffsetDateTime::now_utc();
    (now - time::Duration::hours(1))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| lorehaven_db::identity::now_rfc3339())
}
