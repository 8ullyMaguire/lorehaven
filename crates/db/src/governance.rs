//! M14 — Governance repository: trust, reports, quorum, appeals, sanctions.

use serde_json::Value;
use sqlx::Row;

use lorehaven_domain::governance::TL_NEW;

use crate::{Backend, Database};

async fn fetch_level_sqlite(
    pool: &sqlx::SqlitePool,
    sql: &str,
    account: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(sql).bind(account).fetch_optional(pool).await?;
    Ok(row.map(|r| r.get::<i64, _>(0)).unwrap_or(TL_NEW))
}

async fn fetch_level_postgres(
    pool: &sqlx::PgPool,
    sql: &str,
    account: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(sql).bind(account).fetch_optional(pool).await?;
    // COUNT() is INT8 on PostgreSQL, so the column is i64, not i32.
    Ok(row.map(|r| r.get::<i64, _>(0)).unwrap_or(TL_NEW))
}

pub async fn trust_for(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let sql = "SELECT level FROM trust_levels WHERE account = ?";
            fetch_level_sqlite(db.sqlite_pool().expect("sqlite"), sql, account).await
        }
        Backend::Postgres => {
            let sql = "SELECT CAST(level AS BIGINT) FROM trust_levels WHERE account = $1";
            fetch_level_postgres(db.postgres_pool().expect("postgres"), sql, account).await
        }
    }
}

pub async fn set_trust(
    db: &Database,
    account: &str,
    level: i64,
    basis: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO trust_levels (account, level, computed_at, basis)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(account) DO UPDATE SET level=excluded.level,
                 computed_at=excluded.computed_at, basis=excluded.basis",
            )
            .bind(account)
            .bind(level)
            .bind(&now)
            .bind(basis)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO trust_levels (account, level, computed_at, basis)
                 VALUES ($1, $2, $3, $4::jsonb)
                 ON CONFLICT(account) DO UPDATE SET level=excluded.level,
                 computed_at=excluded.computed_at, basis=excluded.basis",
            )
            .bind(account)
            .bind(level as i32)
            .bind(&now)
            .bind(
                serde_json::from_str::<serde_json::Value>(basis).unwrap_or(serde_json::Value::Null),
            )
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

pub async fn open_report(
    db: &Database,
    subject_type: &str,
    subject_id: &str,
    reporter: &str,
    reason: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let existing = sqlx::query(
                "SELECT id FROM reports WHERE reporter = ? AND subject_id = ? AND state = 'open'",
            )
            .bind(reporter)
            .bind(subject_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?;
            if let Some(row) = existing {
                return Ok(row.get::<String, _>(0));
            }
            sqlx::query(
                "INSERT INTO reports (id, subject_type, subject_id, reporter, reason, created_at, state, resolution)
                 VALUES (?, ?, ?, ?, ?, ?, 'open', '')",
            )
            .bind(&id).bind(subject_type).bind(subject_id).bind(reporter).bind(reason).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            let existing = sqlx::query(
                "SELECT id FROM reports WHERE reporter = $1 AND subject_id = $2 AND state = 'open'",
            )
            .bind(reporter)
            .bind(subject_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?;
            if let Some(row) = existing {
                return Ok(row.get::<String, _>(0));
            }
            sqlx::query(
                "INSERT INTO reports (id, subject_type, subject_id, reporter, reason, created_at, state, resolution)
                 VALUES ($1, $2, $3, $4, $5, $6, 'open', '')",
            )
            .bind(&id).bind(subject_type).bind(subject_id).bind(reporter).bind(reason).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn list_open_reports(db: &Database, limit: i64) -> Result<Vec<Value>, sqlx::Error> {
    let mut items = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(
                "SELECT id, subject_type, subject_id, reporter, reason, created_at, state, resolution
                 FROM reports WHERE state = 'open' ORDER BY created_at LIMIT ?",
            )
            .bind(limit).fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            for r in rows {
                items.push(serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "subject_type": r.get::<String, _>("subject_type"),
                    "subject_id": r.get::<String, _>("subject_id"),
                    "reporter": r.get::<String, _>("reporter"),
                    "reason": r.get::<String, _>("reason"),
                    "created_at": r.get::<String, _>("created_at"),
                    "state": r.get::<String, _>("state"),
                    "resolution": r.get::<String, _>("resolution"),
                }));
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(
                "SELECT id, subject_type, subject_id, reporter, reason, created_at, state, resolution
                 FROM reports WHERE state = 'open' ORDER BY created_at LIMIT $1",
            )
            .bind(limit).fetch_all(db.postgres_pool().expect("postgres")).await?;
            for r in rows {
                items.push(serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "subject_type": r.get::<String, _>("subject_type"),
                    "subject_id": r.get::<String, _>("subject_id"),
                    "reporter": r.get::<String, _>("reporter"),
                    "reason": r.get::<String, _>("reason"),
                    "created_at": r.get::<String, _>("created_at"),
                    "state": r.get::<String, _>("state"),
                    "resolution": r.get::<String, _>("resolution"),
                }));
            }
        }
    }
    Ok(items)
}

pub async fn assign_task(
    db: &Database,
    report_id: &str,
    reviewer: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT reporter FROM reports WHERE id = ?")
                .bind(report_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            if let Some(r) = row {
                let reporter: String = r.get("reporter");
                if reviewer == reporter {
                    return Err(sqlx::Error::Protocol("self-review not allowed".into()));
                }
            }
            sqlx::query("INSERT INTO review_tasks (id, reviewer, report_id, assigned_at, outcome) VALUES (?, ?, ?, ?, 'recuse')")
                .bind(&id).bind(reviewer).bind(report_id).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT reporter FROM reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            if let Some(r) = row {
                let reporter: String = r.get("reporter");
                if reviewer == reporter {
                    return Err(sqlx::Error::Protocol("self-review not allowed".into()));
                }
            }
            sqlx::query("INSERT INTO review_tasks (id, reviewer, report_id, assigned_at, outcome) VALUES ($1, $2, $3, $4, 'recuse')")
                .bind(&id).bind(reviewer).bind(report_id).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn decide_task(
    db: &Database,
    task_id: &str,
    reviewer: &str,
    outcome: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "UPDATE review_tasks SET outcome = ?, decided_at = ? WHERE id = ? AND reviewer = ?",
            )
            .bind(outcome)
            .bind(&now)
            .bind(task_id)
            .bind(reviewer)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE review_tasks SET outcome = $1, decided_at = $2 WHERE id = $3 AND reviewer = $4")
                .bind(outcome).bind(&now).bind(task_id).bind(reviewer)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn issue_sanction(
    db: &Database,
    account: &str,
    kind: &str,
    reason_ref: &str,
    issued_by: &str,
    ends_at: Option<&str>,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO sanctions (id, account, kind, reason_ref, starts_at, ends_at, issued_by)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id).bind(account).bind(kind).bind(reason_ref).bind(&now).bind(ends_at).bind(issued_by)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO sanctions (id, account, kind, reason_ref, starts_at, ends_at, issued_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&id).bind(account).bind(kind).bind(reason_ref).bind(&now).bind(ends_at).bind(issued_by)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn lift_sanction(
    db: &Database,
    sanction_id: &str,
    lifted_by: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE sanctions SET lifted_at = ?, lifted_by = ? WHERE id = ?")
                .bind(&now)
                .bind(lifted_by)
                .bind(sanction_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE sanctions SET lifted_at = $1, lifted_by = $2 WHERE id = $3")
                .bind(&now)
                .bind(lifted_by)
                .bind(sanction_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

pub async fn active_sanctions(db: &Database, account: &str) -> Result<Vec<Value>, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let mut items = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(
                "SELECT id, kind, reason_ref, starts_at, ends_at, issued_by FROM sanctions
                 WHERE account = ? AND lifted_at IS NULL AND (ends_at IS NULL OR ends_at > ?)",
            )
            .bind(account)
            .bind(&now)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?;
            for r in rows {
                items.push(serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "kind": r.get::<String, _>("kind"),
                    "reason_ref": r.get::<String, _>("reason_ref"),
                    "starts_at": r.get::<String, _>("starts_at"),
                    "ends_at": r.get::<String, _>("ends_at"),
                    "issued_by": r.get::<String, _>("issued_by"),
                }));
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(
                "SELECT id, kind, reason_ref, starts_at, ends_at, issued_by FROM sanctions
                 WHERE account = $1 AND lifted_at IS NULL AND (ends_at IS NULL OR ends_at > now())",
            )
            .bind(account)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?;
            for r in rows {
                items.push(serde_json::json!({
                    "id": r.get::<String, _>("id"),
                    "kind": r.get::<String, _>("kind"),
                    "reason_ref": r.get::<String, _>("reason_ref"),
                    "starts_at": r.get::<String, _>("starts_at"),
                    "ends_at": r.get::<String, _>("ends_at"),
                    "issued_by": r.get::<String, _>("issued_by"),
                }));
            }
        }
    }
    Ok(items)
}

pub async fn open_appeal(
    db: &Database,
    sanction_id: &str,
    appellant: &str,
    statement: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let existing =
                sqlx::query("SELECT id FROM appeals WHERE sanction_id = ? AND state = 'open'")
                    .bind(sanction_id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite"))
                    .await?;
            if let Some(row) = existing {
                return Ok(row.get::<String, _>(0));
            }
            sqlx::query(
                "INSERT INTO appeals (id, sanction_id, appellant, statement, created_at, state, decision, decided_by)
                 VALUES (?, ?, ?, ?, ?, 'open', 'upheld', '')",
            )
            .bind(&id).bind(sanction_id).bind(appellant).bind(statement).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            let existing =
                sqlx::query("SELECT id FROM appeals WHERE sanction_id = $1 AND state = 'open'")
                    .bind(sanction_id)
                    .fetch_optional(db.postgres_pool().expect("postgres"))
                    .await?;
            if let Some(row) = existing {
                return Ok(row.get::<String, _>(0));
            }
            sqlx::query(
                "INSERT INTO appeals (id, sanction_id, appellant, statement, created_at, state, decision, decided_by)
                 VALUES ($1, $2, $3, $4, $5, 'open', 'upheld', '')",
            )
            .bind(&id).bind(sanction_id).bind(appellant).bind(statement).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn decide_appeal(
    db: &Database,
    appeal_id: &str,
    decision: &str,
    decided_by: &str,
) -> Result<(), sqlx::Error> {
    // §19.4: the decision-maker must not be the appellant — independence is
    // a hard rule, not a policy toggle.
    let appellant: Option<String> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar("SELECT appellant FROM appeals WHERE id = ?")
                .bind(appeal_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar("SELECT appellant FROM appeals WHERE id = $1")
                .bind(appeal_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    if appellant.as_deref() == Some(decided_by) {
        return Err(sqlx::Error::Protocol(
            "appeal decision must be made by a different account than the appellant".into(),
        ));
    }
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE appeals SET decision = ?, decided_by = ?, decided_at = ?, state = 'decided' WHERE id = ?")
                .bind(decision).bind(decided_by).bind(&now).bind(appeal_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE appeals SET decision = $1, decided_by = $2, decided_at = $3, state = 'decided' WHERE id = $4")
                .bind(decision).bind(decided_by).bind(&now).bind(appeal_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn audit_append(
    db: &Database,
    actor: &str,
    action: &str,
    subject_type: &str,
    subject_id: &str,
    document: &str,
) -> Result<(), sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO audit_log (id, actor, action, subject_type, subject_id, document, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id).bind(actor).bind(action).bind(subject_type).bind(subject_id).bind(document).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO audit_log (id, actor, action, subject_type, subject_id, document, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7)",
            )
            .bind(&id).bind(actor).bind(action).bind(subject_type).bind(subject_id)
            .bind(serde_json::from_str::<serde_json::Value>(document).unwrap_or(serde_json::Value::Null))
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn has_operator_role(db: &Database, account: &str) -> Result<bool, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query("SELECT 1 FROM operator_role WHERE account = ?")
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.is_some())
        }
        Backend::Postgres => {
            let row = sqlx::query("SELECT 1 FROM operator_role WHERE account = $1")
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.is_some())
        }
    }
}

/// Operator check used by admin routes (spec §19.1): trust level >= 5.
pub async fn is_operator(db: &Database, account: &str) -> Result<bool, sqlx::Error> {
    let level = trust_for(db, account).await?;
    Ok(level >= 5)
}

pub async fn grant_operator_role(
    db: &Database,
    account: &str,
    role: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO operator_role (account, role, granted_at) VALUES (?, ?, ?)
                 ON CONFLICT(account) DO UPDATE SET role=excluded.role, granted_at=excluded.granted_at",
            )
            .bind(account).bind(role).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO operator_role (account, role, granted_at) VALUES ($1, $2, $3)
                 ON CONFLICT(account) DO UPDATE SET role=excluded.role, granted_at=excluded.granted_at",
            )
            .bind(account).bind(role).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Appeal listing
// ---------------------------------------------------------------------------

pub async fn list_appeals(db: &Database, account: &str) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT id, sanction_id, appellant, statement, created_at, state, decided_at, decision, decided_by FROM appeals WHERE appellant = ? ORDER BY created_at DESC")
                .bind(account)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "sanction_id": r.get::<String, _>("sanction_id"),
                        "appellant": r.get::<String, _>("appellant"),
                        "statement": r.get::<String, _>("statement"),
                        "created_at": r.get::<String, _>("created_at"),
                        "state": r.get::<String, _>("state"),
                        "decided_at": r.get::<Option<String>, _>("decided_at"),
                        "decision": r.get::<String, _>("decision"),
                        "decided_by": r.get::<String, _>("decided_by"),
                    })
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query("SELECT id, sanction_id, appellant, statement, created_at, state, decided_at, decision, decided_by FROM appeals WHERE appellant = $1 ORDER BY created_at DESC")
                .bind(account)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "sanction_id": r.get::<String, _>("sanction_id"),
                        "appellant": r.get::<String, _>("appellant"),
                        "statement": r.get::<String, _>("statement"),
                        "created_at": r.get::<String, _>("created_at"),
                        "state": r.get::<String, _>("state"),
                        "decided_at": r.get::<Option<String>, _>("decided_at"),
                        "decision": r.get::<String, _>("decision"),
                        "decided_by": r.get::<String, _>("decided_by"),
                    })
                })
                .collect())
        }
    }
}

// ---------------------------------------------------------------------------
// Audit log listing
// ---------------------------------------------------------------------------

pub async fn list_audit_log(
    db: &Database,
    account_id: &str,
    limit: i64,
) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query("SELECT id, actor, action, subject_type, subject_id, document, created_at FROM audit_log WHERE actor = ? ORDER BY created_at DESC LIMIT ?")
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "actor": r.get::<String, _>("actor"),
                        "action": r.get::<String, _>("action"),
                        "subject_type": r.get::<String, _>("subject_type"),
                        "subject_id": r.get::<String, _>("subject_id"),
                        "document": r.get::<String, _>("document"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
        Backend::Postgres => {
            // `audit_log.document` is `TEXT` in SQLite and `JSONB` in PostgreSQL
            // (0016_governance.sql). The row is handed to a `String` reader, so
            // the column is cast to text on that side only -- otherwise
            // `r.get::<String>("document")` is a decode error and the whole audit
            // trail 500s. The write side already binds a `Value` into JSONB.
            let rows = sqlx::query("SELECT id, actor, action, subject_type, subject_id, document::text AS document, created_at FROM audit_log WHERE actor = $1 ORDER BY created_at DESC LIMIT $2")
                .bind(account_id)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "actor": r.get::<String, _>("actor"),
                        "action": r.get::<String, _>("action"),
                        "subject_type": r.get::<String, _>("subject_type"),
                        "subject_id": r.get::<String, _>("subject_id"),
                        "document": r.get::<String, _>("document"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect())
        }
    }
}
