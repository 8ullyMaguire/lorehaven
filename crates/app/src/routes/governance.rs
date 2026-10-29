//! M14 — Governance routes: reports, moderation, sanctions, appeals, trust.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use darkhold_domain::AppError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{MaybeSession, RequireSession};
use crate::format_rfc3339;
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    routes()
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/reports", post(submit_report))
        .route("/reports", get(list_reports))
        .route("/reports/{id}", get(get_report))
        .route("/moderation/queue", get(moderation_queue))
        .route("/moderation/reports/{id}/assign", post(assign_task))
        .route("/moderation/tasks/{id}/decide", post(decide_task))
        .route("/sanctions", post(issue_sanction))
        // Two-phase for the IRREVERSIBLE case (capability ledger 3.4): filing creates a pending
        // case and changes nothing, executing applies it. Ported from gravity's
        // `/discipline/permanent-ban` + `/permanent-ban/execute` pair.
        .route("/sanctions/file", post(file_sanction))
        .route("/sanctions/{id}/execute", post(execute_sanction))
        .route("/sanctions/{id}/lift", post(lift_sanction))
        .route("/me/sanctions", get(my_sanctions))
        .route("/appeals", post(open_appeal))
        .route("/appeals/{id}/decide", post(decide_appeal))
        .route("/appeals", get(list_my_appeals))
        .route("/me/trust", get(my_trust))
        .route("/me/audit-log", get(my_audit_log))
        // The modlog: readable by any signed-in reader (spec §14.6).
        .route("/modlog", get(modlog))
        // An account's trust history: the events, not just the level.
        .route("/accounts/{id}/trust", get(account_trust_history))
}

// ---------------------------------------------------------------------------
// Reports (spec §19.3)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SubmitReportBody {
    subject_type: String,
    subject_id: String,
    reason: String,
    detail: Option<String>,
}

async fn submit_report(
    State(state): State<AppState>,
    MaybeSession(user): MaybeSession,
    Json(body): Json<SubmitReportBody>,
) -> ApiResult<Json<Value>> {
    // Reports may be submitted by anyone, including unauthenticated visitors.
    // Anonymous reports are attributed to a sentinel account id "anonymous".
    let reporter = user
        .as_ref()
        .map(|u| u.account_id.to_string())
        .unwrap_or_else(|| "anonymous".to_owned());
    let id = darkhold_db::governance::open_report(
        state.db(),
        &body.subject_type,
        &body.subject_id,
        &reporter,
        &body.reason,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &reporter,
            // A report may be filed by an anonymous visitor, and then there is
            // no pseudonym behind it. An honest absence beats a made-up one.
            actor_pseud: user
                .as_ref()
                .and_then(|u| u.pseud_id.map(|id| id.to_string()))
                .as_deref(),
            reason: Some(&body.reason),
            action: "report.submitted",
            subject_type: &body.subject_type,
            subject_id: &body.subject_id,
            document: &json!({ "reason": body.reason, "detail": body.detail, "report_id": id })
                .to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

async fn list_reports(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    let items = darkhold_db::governance::list_open_reports(state.db(), 100)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}

async fn get_report(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RequireSession(_user): RequireSession,
) -> ApiResult<Json<Value>> {
    let items = darkhold_db::governance::list_open_reports(state.db(), 1000)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let found = items.into_iter().find(|r| r["id"].as_str() == Some(&id));
    match found {
        Some(r) => Ok(Json(r)),
        None => Err(ApiError(AppError::field("report", "not found"))),
    }
}

// ---------------------------------------------------------------------------
// Moderation queue (spec §19.4)
// ---------------------------------------------------------------------------

async fn moderation_queue(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let reviewer = user.account_id.to_string();
    // Spec §14.4: the moderation queue is a **moderator's** privilege. The
    // route asks for the privilege and does not know what level that is.
    let (level, required) = crate::auth::privilege_check(
        &state,
        &reviewer,
        darkhold_domain::trust::Privilege::ModerationQueue,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < required {
        return Err(ApiError(AppError::TrustLevelInsufficient {
            level,
            required,
        }));
    }
    let items = darkhold_db::governance::list_open_reports(state.db(), 100)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct AssignTaskBody {}

async fn assign_task(
    State(state): State<AppState>,
    Path(report_id): Path<String>,
    RequireSession(user): RequireSession,
    _body: Json<AssignTaskBody>,
) -> ApiResult<Json<Value>> {
    let reviewer = user.account_id.to_string();
    // Assigning a report is **holding the moderation queue** — the same
    // privilege that reads it. The gate was missing here, which meant any
    // signed-in reader could put their name on a review task.
    let (level, required) = crate::auth::privilege_check(
        &state,
        &reviewer,
        darkhold_domain::trust::Privilege::ModerationQueue,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < required {
        return Err(ApiError(AppError::TrustLevelInsufficient {
            level,
            required,
        }));
    }
    let task_id = darkhold_db::governance::assign_task(state.db(), &report_id, &reviewer)
        .await
        .map_err(|e| {
            if e.to_string().contains("self-review") {
                AppError::field("reviewer", "self-review not allowed")
            } else {
                AppError::Internal(e.into())
            }
        })?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &reviewer,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: None,
            action: "review.assigned",
            subject_type: "report",
            subject_id: &report_id,
            document: &json!({ "task_id": task_id }).to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "task_id": task_id })))
}

#[derive(Debug, Deserialize)]
struct DecideTaskBody {
    outcome: String,
}

async fn decide_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    RequireSession(user): RequireSession,
    Json(body): Json<DecideTaskBody>,
) -> ApiResult<Json<Value>> {
    let reviewer = user.account_id.to_string();
    // Spec §14.4: **resolving a report** is a senior reviewer's privilege.
    // The gate was missing here, which meant any signed-in reader could
    // uphold or dismiss a report.
    let (level, required) = crate::auth::privilege_check(
        &state,
        &reviewer,
        darkhold_domain::trust::Privilege::ResolveReports,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < required {
        return Err(ApiError(AppError::TrustLevelInsufficient {
            level,
            required,
        }));
    }
    darkhold_db::governance::decide_task(state.db(), &task_id, &reviewer, &body.outcome)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &reviewer,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            // The outcome **is** the reason: a review was decided, and what it
            // was decided as is why.
            reason: Some(&body.outcome),
            action: "review.decided",
            subject_type: "task",
            subject_id: &task_id,
            document: &json!({ "outcome": body.outcome }).to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// Sanctions (spec §19.5)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct IssueSanctionBody {
    account: String,
    kind: String,
    reason_ref: String,
    duration_days: Option<i64>,
}

async fn issue_sanction(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<IssueSanctionBody>,
) -> ApiResult<Json<Value>> {
    let issuer = user.account_id.to_string();
    require_scoped_bans(&state, &issuer).await?;

    // THE IRREVERSIBLE CASE IS NOT ONE CALL (ledger 3.4).
    //
    // A sanction with no `duration_days` never expires, and `lift` is the only thing that can
    // undo it. Sending it through this one-shot route is exactly the accident the two-phase
    // exists to prevent, so it is refused with a message that names the route to use instead —
    // a refusal a caller cannot read is indistinguishable from a bug.
    if body.duration_days.is_none() {
        return Err(ApiError(AppError::field(
            "duration_days",
            "a sanction with no expiry is irreversible, so it takes two calls: POST /sanctions/file, then POST /sanctions/{id}/execute with the returned id",
        )));
    }

    let ends_at = body.duration_days.map(|d| {
        let end = time::OffsetDateTime::now_utc() + time::Duration::days(d);
        format_rfc3339(end)
    });

    let id = darkhold_db::governance::issue_sanction(
        state.db(),
        &body.account,
        &body.kind,
        &body.reason_ref,
        &issuer,
        ends_at.as_deref(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &issuer,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            // A sanction without a reason is a sanction nobody can contest.
            reason: Some(&body.reason_ref),
            action: "sanction.issued",
            subject_type: "account",
            subject_id: &body.account,
            document: &json!({ "sanction_id": id, "kind": body.kind, "days": body.duration_days })
                .to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

/// The one door for "may this account issue a scoped ban?" (spec §14.4).
///
/// A helper rather than three copies: the two-phase routes and the one-shot route must not be
/// able to drift into different privilege requirements, and the drift would be invisible —
/// each handler reads correctly on its own.
async fn require_scoped_bans(state: &AppState, issuer: &str) -> ApiResult<()> {
    // Spec §14.4: a **scoped ban** is a senior reviewer's privilege.
    //
    // **The nominal level is 5 and the elastic allocation is what keeps a small site governable**
    // (spec §14.4): on an instance with no senior reviewer the requirement falls to the level that
    // actually has somebody in it, and it returns to 5 on the next evaluation once a senior
    // reviewer appears.
    let (level, required) = crate::auth::privilege_check(
        state,
        issuer,
        darkhold_domain::trust::Privilege::ScopedBans,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < required {
        return Err(ApiError(AppError::TrustLevelInsufficient { level, required }));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct FileSanctionBody {
    account: String,
    kind: String,
    reason_ref: String,
    /// Absent means permanent. That is allowed HERE and refused on the one-shot route: filing a
    /// permanent case is reversible (a pending case bites nobody), executing it is not.
    duration_days: Option<i64>,
}

/// `POST /sanctions/file` — phase one. Audited, pending, and inert.
async fn file_sanction(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<FileSanctionBody>,
) -> ApiResult<Json<Value>> {
    let filer = user.account_id.to_string();
    require_scoped_bans(&state, &filer).await?;

    let ends_at = body.duration_days.map(|d| {
        let end = time::OffsetDateTime::now_utc() + time::Duration::days(d);
        format_rfc3339(end)
    });

    let id = darkhold_db::governance::file_sanction(
        state.db(),
        &body.account,
        &body.kind,
        &body.reason_ref,
        &filer,
        ends_at.as_deref(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &filer,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: Some(&body.reason_ref),
            action: "sanction.filed",
            subject_type: "account",
            subject_id: &body.account,
            document: &json!({
                "sanction_id": id,
                "kind": body.kind,
                "days": body.duration_days,
                "state": "pending",
            })
            .to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "id": id,
        "state": "pending",
        // Said out loud because a client that treats this as done will never call execute, and the
        // account would stay unsanctioned while the operator believes otherwise.
        "next": "POST /sanctions/{id}/execute",
    })))
}

/// `POST /sanctions/{id}/execute` — phase two. Applies a filed case, once.
async fn execute_sanction(
    State(state): State<AppState>,
    Path(sanction_id): Path<String>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let executor = user.account_id.to_string();
    require_scoped_bans(&state, &executor).await?;

    let applied = darkhold_db::governance::execute_sanction(state.db(), &sanction_id, &executor)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    if !applied {
        // Not pending: unknown, already executed, or lifted before it ran. A conflict rather than a
        // quiet success, because "executed" is a claim an operator will rely on.
        let state_now = darkhold_db::governance::sanction_state(state.db(), &sanction_id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;
        return Err(ApiError(AppError::field(
            "sanction",
            match state_now {
                Some(s) => format!(
                    "sanction {sanction_id} is not pending (state: {s}); it was already executed or lifted, and executing twice would record an action that changed nothing"
                ),
                None => format!("sanction {sanction_id} does not exist"),
            },
        )));
    }

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &executor,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: Some("executed a filed sanction"),
            action: "sanction.executed",
            subject_type: "sanction",
            subject_id: &sanction_id,
            document: &json!({ "state": "active" }).to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": sanction_id, "state": "active" })))
}

async fn lift_sanction(
    State(state): State<AppState>,
    Path(sanction_id): Path<String>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let lifter = user.account_id.to_string();
    // Lifting a ban is the same authority as issuing one.
    let (level, required) = crate::auth::privilege_check(
        &state,
        &lifter,
        darkhold_domain::trust::Privilege::ScopedBans,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    if level < required {
        return Err(ApiError(AppError::TrustLevelInsufficient {
            level,
            required,
        }));
    }
    darkhold_db::governance::lift_sanction(state.db(), &sanction_id, &lifter)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &lifter,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: None,
            action: "sanction.lifted",
            subject_type: "sanction",
            subject_id: &sanction_id,
            document: "{}",
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "ok": true })))
}

async fn my_sanctions(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let items = darkhold_db::governance::active_sanctions(state.db(), &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}

// ---------------------------------------------------------------------------
// Appeals (spec §19.10)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenAppealBody {
    sanction_id: String,
    statement: String,
}

async fn open_appeal(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<OpenAppealBody>,
) -> ApiResult<Json<Value>> {
    let appellant = user.account_id.to_string();
    let id = darkhold_db::governance::open_appeal(
        state.db(),
        &body.sanction_id,
        &appellant,
        &body.statement,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &appellant,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: Some(&body.statement),
            action: "appeal.opened",
            subject_type: "sanction",
            subject_id: &body.sanction_id,
            document: &json!({ "appeal_id": id }).to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "id": id })))
}

#[derive(Debug, Deserialize)]
struct DecideAppealBody {
    decision: String,
}

async fn decide_appeal(
    State(state): State<AppState>,
    Path(appeal_id): Path<String>,
    RequireSession(user): RequireSession,
    Json(body): Json<DecideAppealBody>,
) -> ApiResult<Json<Value>> {
    let decider = user.account_id.to_string();

    // The decider must not be the issuer of the sanction (independence).
    // Independence checking against the audit log lands with the M14
    // quorum work; for now the decision is recorded as given.
    darkhold_db::governance::decide_appeal(state.db(), &appeal_id, &body.decision, &decider)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    darkhold_db::governance::audit_append(
        state.db(),
        &darkhold_db::governance::AuditEntry {
            actor: &decider,
            actor_pseud: user.pseud_id.map(|id| id.to_string()).as_deref(),
            reason: Some(&body.decision),
            action: "appeal.decided",
            subject_type: "appeal",
            subject_id: &appeal_id,
            document: &json!({ "decision": body.decision }).to_string(),
        },
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({ "ok": true })))
}

async fn list_my_appeals(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let appeals = darkhold_db::governance::list_appeals(state.db(), &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": appeals })))
}

// ---------------------------------------------------------------------------
// Trust (spec §19.1)
// ---------------------------------------------------------------------------

/// The modlog: every moderator, curator and operator action, newest first
/// (spec §14.6).
///
/// **Anonymous access is 401, not 403.** "403 answers 'yes, and you may not'"
/// — but the modlog is not a door that exists only for the permitted. It is a
/// public record that any reader of the instance may read, and being
/// anonymous is a fact about the *reader* rather than about the record. A 403
/// would tell an anonymous caller that the modlog exists and that they may
/// not see it, which is an answer to a question the specification says they
/// may ask.
///
/// **The acting account is never published.** The actor is the acting
/// pseudonym, and a subject that is an account is published as that account's
/// public face. The account id stays where it belongs: in the instance's own
/// audit.
async fn modlog(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Query(q): Query<ModlogQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let after = q.cursor.as_deref().map(crate::cursor::decode).transpose()?;
    let entries = darkhold_db::governance::list_modlog(
        state.db(),
        limit,
        after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let next_cursor = if i64::try_from(entries.len()).unwrap_or(i64::MAX) >= limit {
        entries
            .last()
            .map(|entry| crate::cursor::encode(&entry.created_at, &entry.id))
    } else {
        None
    };

    Ok(Json(json!({
        "items": entries
            .into_iter()
            .map(|entry| json!({
                "id": entry.id,
                "actor": {
                    "pseud_id": entry.actor_pseud,
                    "handle": entry.actor_handle,
                    // A panel decision is the instance acting, not a person.
                    "is_panel": entry.actor_is_panel,
                },
                "action": entry.action,
                "target": {
                    "type": entry.subject_type,
                    "id": entry.subject_id,
                    // True when the target was an account and was published as
                    // a pseudonym instead.
                    "is_pseudonym": entry.subject_is_pseudonym,
                },
                "reason": entry.reason,
                "created_at": entry.created_at,
            }))
            .collect::<Vec<_>>(),
        "next_cursor": next_cursor,
    })))
}

/// The query the modlog takes.
#[derive(Debug, Deserialize)]
pub struct ModlogQuery {
    /// How many entries to return.
    pub limit: Option<i64>,
    /// Which page: the cursor the previous page handed back.
    pub cursor: Option<String>,
}

/// An account's trust history: the events, not just the level (spec §6.7).
///
/// **The level is the headline and the history is the answer.** A reader who
/// wants to know why their level is what it is needs the events, and an
/// operator who wants to know why somebody's level fell needs the same events.
async fn account_trust_history(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Path(id): Path<String>,
    Query(q): Query<ModlogQuery>,
) -> ApiResult<Json<Value>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let events = darkhold_db::trust_events::list_for_account(state.db(), &id, limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    let level = darkhold_db::governance::trust_for(state.db(), &id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    Ok(Json(json!({
        "account_id": id,
        "level": level,
        "level_name": state.config().trust.ladder().name_of(level),
        "events": events
            .into_iter()
            .map(|event| json!({
                "id": event.id,
                "from_level": event.from_level,
                "to_level": event.to_level,
                "cause": event.cause.as_str(),
                "evidence_ref": event.evidence_ref,
                "created_at": event.created_at,
            }))
            .collect::<Vec<_>>(),
    })))
}

async fn my_trust(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let account = user.account_id.to_string();
    let level = darkhold_db::governance::trust_for(state.db(), &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let description = match level {
        0 => "New account",
        1 => "Established participant",
        2 => "Regular participant",
        3 => "Reviewed trusted contributor",
        4 => "Trained steward",
        5 => "Senior independent reviewer",
        6 => "Explicitly appointed trustee",
        _ => "Unknown",
    };

    Ok(Json(json!({ "level": level, "description": description })))
}

async fn my_audit_log(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<Value>> {
    let items =
        darkhold_db::governance::list_audit_log(state.db(), &user.account_id.to_string(), 50)
            .await
            .map_err(|e| ApiError(darkhold_domain::AppError::Internal(e.into())))?;
    Ok(Json(json!({ "items": items })))
}
