//! M29 — Recommendation transparency: "why am I seeing this" explanations.
//!
//! ```text
//! GET  /discovery/slots/{slot_id}/explanation  why a recommendation appears
//! GET  /me/attention-report                  private reading attention report
//! POST /admin/tag-wrangling/proposals         propose a tag merge/alias
//! GET  /admin/tag-wrangling/proposals         list pending proposals
//! POST /admin/tag-wrangling/proposals/:id/approve  approve a proposal
//! ```

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::RequireSession;
use crate::http::ApiResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery/slots/{slot_id}/explanation", get(explain_slot))
        .route("/me/attention-report", get(get_attention_report))
        .route("/admin/tag-wrangling/proposals", post(propose_wrangling))
        .route(
            "/admin/tag-wrangling/proposals",
            get(list_wrangling_proposals),
        )
        .route(
            "/admin/tag-wrangling/proposals/{id}/approve",
            post(approve_wrangling),
        )
}

#[derive(Debug, Serialize)]
struct SlotExplanation {
    slot_id: String,
    reasons: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    taste_signal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seeded_by: Option<String>,
    /// Instance curation is always shown as one undifferentiated line.
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_curation: Option<String>,
}

async fn explain_slot(
    State(_state): State<AppState>,
    Path(slot_id): Path<String>,
) -> ApiResult<Json<SlotExplanation>> {
    Ok(Json(SlotExplanation {
        slot_id,
        reasons: vec!["taste_signal: matching your reading history".to_string()],
        taste_signal: Some("matching tags in your taste profile".to_string()),
        filter: None,
        recipe_stage: None,
        seeded_by: None,
        instance_curation: None,
    }))
}

#[derive(Debug, Serialize)]
struct AttentionReport {
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lines: Option<Vec<String>>,
}

async fn get_attention_report(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<AttentionReport>> {
    Ok(Json(AttentionReport {
        enabled: false,
        lines: None,
    }))
}

#[derive(Debug, Deserialize)]
pub struct WranglingProposal {
    pub kind: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub reason: String,
}

async fn propose_wrangling(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Json(_body): Json<WranglingProposal>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "id": "proposal-id", "status": "pending" })))
}

async fn list_wrangling_proposals(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "items": [] })))
}

async fn approve_wrangling(
    State(_state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "id": id, "status": "approved" })))
}
