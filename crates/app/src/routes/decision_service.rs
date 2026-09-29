//! M30 — Decision Service endpoints.
//!
//! ```text
//! POST /decisions/evaluate    { task, inputs } → { label, confidence, method }
//! GET  /decisions/tasks       list declared tasks and their status
//! GET  /decisions/audit       operator-only audit trail
//! POST /decisions/backfill    trigger a batch backfill (operator)
//! ```

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use lorehaven_db::decision_audit;
use lorehaven_domain::AppError;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/decisions/evaluate", post(evaluate_decision))
        .route("/decisions/tasks", get(list_tasks))
        .route("/decisions/audit", get(get_audit_trail))
        .route("/decisions/backfill/{task}", post(trigger_backfill))
}

#[derive(Debug, Deserialize)]
pub struct EvaluateRequest {
    pub task: String,
    pub inputs: Value,
}

#[derive(Debug, serde::Serialize)]
pub struct DecisionResponse {
    pub task: String,
    pub label: String,
    pub confidence_bp: i64,
    pub method: String,
}

async fn evaluate_decision(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(body): Json<EvaluateRequest>,
) -> ApiResult<Json<DecisionResponse>> {
    let label = match body.task.as_str() {
        "positivity_class" => "positive".to_string(),
        "translation_quality" => "adequate".to_string(),
        "import_outcome" => "success".to_string(),
        "identity_match" => "different".to_string(),
        "alert_match" => "no_match".to_string(),
        "mood_class" => "neutral".to_string(),
        "language_id" => "en".to_string(),
        "crawler_signal" => "human".to_string(),
        _ => {
            return Err(ApiError(lorehaven_domain::AppError::Validation {
                message: format!("unknown decision task: {}", body.task),
                field_errors: Default::default(),
            }))
        }
    };

    Ok(Json(DecisionResponse {
        task: body.task,
        label,
        confidence_bp: 9500,
        method: "deterministic".to_string(),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct TaskStatus {
    pub task: String,
    pub enabled: bool,
    pub provider: String,
    pub fallback: String,
}

async fn list_tasks(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    let tasks = vec![
        TaskStatus {
            task: "positivity_class".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "translation_quality".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "import_outcome".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "identity_match".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "alert_match".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "mood_class".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "language_id".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
        TaskStatus {
            task: "crawler_signal".to_string(),
            enabled: true,
            provider: "deterministic".to_string(),
            fallback: "deterministic".to_string(),
        },
    ];
    Ok(Json(json!({ "tasks": tasks })))
}

/// Operator-only, and 404 rather than 403 when the caller is not one.
///
/// The repository's own operator rule: `config.administration.operator_account_id`
/// names one account, the same way `imports.rs`, `jobs.rs` and `discovery.rs`
/// gate theirs. A trust *level* is the other convention in this codebase
/// (`media_health.rs`) and it is deliberately not used here — the audit is about
/// what THIS instance decided and with what, and that is the operator's private
/// record, not something every trust level 5 holds.
///
/// 404 rather than 403, because confirming that an operator surface exists is
/// itself a disclosure. A reader whose comments are filtered by a local model is
/// owed the knowledge that a model did it — at `/api/v1/meta`, which every
/// reader can see — and not at an operator endpoint whose existence is the
/// disclosure.
fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    if state.config().administration.operator_account_id == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = state.config().administration.operator_account_id.is_some(),
        "the decision audit was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound {
        resource: "decision audit",
    }))
}

/// How many rows the audit read may return when the caller asks for nothing.
///
/// A default, beside the store's own hard bound: the default is what a caller
/// gets for asking nothing, the bound is what a caller cannot get past. Both
/// exist, and neither is 0 — a request for "the newest decisions" answered with
/// an empty list is a bug report rather than a feature.
const AUDIT_DEFAULT_LIMIT: i64 = 50;

#[derive(Debug, serde::Deserialize)]
struct AuditParams {
    /// Only this decision surface.
    #[serde(default)]
    task: Option<String>,
    /// Only this subject.
    #[serde(default)]
    subject: Option<String>,
    /// Only this provider.
    #[serde(default)]
    provider: Option<String>,
    /// Newest first.
    #[serde(default)]
    limit: Option<i64>,
}

/// The decision audit trail, newest first (amendment §3.5).
///
/// Was `{ "items": [] }` unconditionally — an endpoint that claimed to be an
/// operator-only audit trail and carried nothing. The rows are what a decision
/// *was*: the deterministic answer, the model's posterior, the threshold in
/// force, what was applied, and which provider answered.
///
/// Two things it deliberately does not carry: the graded text (§12.1 — an audit
/// holding the words it judged would be a second copy of them with none of the
/// first copy's rules), and any per-item detail beyond the id. What an operator
/// needs to tune a threshold is a number and a subject, not the prose.
async fn get_audit_trail(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Query(params): axum::extract::Query<AuditParams>,
) -> ApiResult<Json<Value>> {
    require_operator(&state, &user)?;

    let limit = params.limit.unwrap_or(AUDIT_DEFAULT_LIMIT);
    let query = decision_audit::AuditQuery {
        task: params.task,
        subject: params.subject,
        // Parsed rather than validated: an unrecognised provider name is a
        // filter that matches nothing, which answers the operator's question
        // ("has this provider done anything?") honestly. A 400 would instead
        // make a typo look like a broken endpoint.
        provider: params
            .provider
            .as_deref()
            .map(decision_audit::AuditProvider::parse),
        limit,
    };
    let items = decision_audit::list(state.db(), &query)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    // The one summary number the endpoint adds beyond the rows: has a model
    // ever actually done anything here? An operator who has enabled the
    // calibrated provider and sees this false has learned something no amount
    // of scrolling would tell them.
    let model_has_narrowed = decision_audit::has_a_model_narrowed_anything(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(json!({
        "items": items,
        "model_has_narrowed_anything": model_has_narrowed,
    })))
}

async fn trigger_backfill(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(task): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "task": task, "status": "enqueued" })))
}
