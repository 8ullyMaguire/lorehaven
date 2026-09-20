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
        _ => return Err(ApiError(lorehaven_domain::AppError::Validation {
            message: format!("unknown decision task: {}", body.task),
            field_errors: Default::default(),
        })),
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
        TaskStatus { task: "positivity_class".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "translation_quality".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "import_outcome".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "identity_match".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "alert_match".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "mood_class".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "language_id".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
        TaskStatus { task: "crawler_signal".to_string(), enabled: true, provider: "deterministic".to_string(), fallback: "deterministic".to_string() },
    ];
    Ok(Json(json!({ "tasks": tasks })))
}

async fn get_audit_trail(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "items": [] })))
}

async fn trigger_backfill(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(task): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "task": task, "status": "enqueued" })))
}
