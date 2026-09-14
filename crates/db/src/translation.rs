//! M17 — Translation repository: jobs, units, memory, glossaries, reviews, publications.

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use lorehaven_domain::translation::{self, TranslationJobState, TranslationUnitState, ReviewGate};
use crate::{Backend, Database};

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

pub async fn create_job(
    db: &Database,
    source_work: &str,
    source_lang: &str,
    target_lang: &str,
    provider: &str,
    quote_transaction: Option<&str>,
    created_by: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_jobs (id, source_work, source_lang, target_lang, provider, quote_transaction, state, created_by, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'quoted', ?, ?, ?)"
            )
            .bind(&id).bind(source_work).bind(source_lang).bind(target_lang).bind(provider).bind(quote_transaction).bind(created_by).bind(&now).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_jobs (id, source_work, source_lang, target_lang, provider, quote_transaction, state, created_by, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'quoted', $7, $8, $9)"
            )
            .bind(&id).bind(source_work).bind(source_lang).bind(target_lang).bind(provider).bind(quote_transaction).bind(created_by).bind(&now).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

async fn get_job_sqlite(pool: &sqlx::SqlitePool, job_id: &str) -> Result<Option<Value>, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM translation_jobs WHERE id = ?")
        .bind(job_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| {
        serde_json::json!({
            "id": r.get::<String, _>("id"),
            "source_work": r.get::<String, _>("source_work"),
            "source_lang": r.get::<String, _>("source_lang"),
            "target_lang": r.get::<String, _>("target_lang"),
            "provider": r.get::<String, _>("provider"),
            "quote_transaction": r.get::<Option<String>, _>("quote_transaction"),
            "state": r.get::<String, _>("state"),
            "created_by": r.get::<String, _>("created_by"),
            "created_at": r.get::<String, _>("created_at"),
            "updated_at": r.get::<String, _>("updated_at"),
        })
    }))
}

async fn get_job_postgres(pool: &sqlx::postgres::PgPool, job_id: &str) -> Result<Option<Value>, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM translation_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| {
        serde_json::json!({
            "id": r.get::<String, _>("id"),
            "source_work": r.get::<String, _>("source_work"),
            "source_lang": r.get::<String, _>("source_lang"),
            "target_lang": r.get::<String, _>("target_lang"),
            "provider": r.get::<String, _>("provider"),
            "quote_transaction": r.get::<Option<String>, _>("quote_transaction"),
            "state": r.get::<String, _>("state"),
            "created_by": r.get::<String, _>("created_by"),
            "created_at": r.get::<String, _>("created_at"),
            "updated_at": r.get::<String, _>("updated_at"),
        })
    }))
}

pub async fn get_job(db: &Database, job_id: &str) -> Result<Option<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => get_job_sqlite(db.sqlite_pool().expect("sqlite"), job_id).await,
        Backend::Postgres => get_job_postgres(db.postgres_pool().expect("postgres"), job_id).await,
    }
}

pub async fn transition_job(
    db: &Database,
    job_id: &str,
    from: &TranslationJobState,
    to: &TranslationJobState,
) -> Result<(), sqlx::Error> {
    if !translation::valid_job_transition(from, to) {
        return Err(sqlx::Error::Protocol(format!("invalid transition: {:?} -> {:?}", from, to)));
    }

    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE translation_jobs SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
                .bind(to.as_str()).bind(&now).bind(job_id).bind(from.as_str())
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE translation_jobs SET state = $1, updated_at = $2 WHERE id = $3 AND state = $4")
                .bind(to.as_str()).bind(&now).bind(job_id).bind(from.as_str())
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

pub async fn upsert_unit(
    db: &Database,
    job_id: &str,
    chapter_id: &str,
    paragraph_index: i64,
    source_text: &str,
    target_text: Option<&str>,
    state: &TranslationUnitState,
    memory_hit: Option<&str>,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_units (id, job_id, chapter_id, paragraph_index, source_text, target_text, state, memory_hit, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(job_id, chapter_id, paragraph_index) DO UPDATE SET
                    target_text = excluded.target_text, state = excluded.state, memory_hit = excluded.memory_hit, updated_at = excluded.updated_at"
            )
            .bind(&id).bind(job_id).bind(chapter_id).bind(paragraph_index).bind(source_text).bind(target_text).bind(state.as_str()).bind(memory_hit).bind(&now).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_units (id, job_id, chapter_id, paragraph_index, source_text, target_text, state, memory_hit, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(job_id, chapter_id, paragraph_index) DO UPDATE SET
                    target_text = EXCLUDED.target_text, state = EXCLUDED.state, memory_hit = EXCLUDED.memory_hit, updated_at = EXCLUDED.updated_at"
            )
            .bind(&id).bind(job_id).bind(chapter_id).bind(paragraph_index).bind(source_text).bind(target_text).bind(state.as_str()).bind(memory_hit).bind(&now).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

async fn units_for_job_sqlite(pool: &sqlx::SqlitePool, job_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM translation_units WHERE job_id = ? ORDER BY chapter_id, paragraph_index")
        .bind(job_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| {
        serde_json::json!({
            "id": r.get::<String, _>("id"),
            "job_id": r.get::<String, _>("job_id"),
            "chapter_id": r.get::<String, _>("chapter_id"),
            "paragraph_index": r.get::<i64, _>("paragraph_index"),
            "source_text": r.get::<String, _>("source_text"),
            "target_text": r.get::<Option<String>, _>("target_text"),
            "state": r.get::<String, _>("state"),
            "memory_hit": r.get::<Option<String>, _>("memory_hit"),
        })
    }).collect())
}

async fn units_for_job_postgres(pool: &sqlx::postgres::PgPool, job_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM translation_units WHERE job_id = $1 ORDER BY chapter_id, paragraph_index")
        .bind(job_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| {
        serde_json::json!({
            "id": r.get::<String, _>("id"),
            "job_id": r.get::<String, _>("job_id"),
            "chapter_id": r.get::<String, _>("chapter_id"),
            "paragraph_index": r.get::<i64, _>("paragraph_index"),
            "source_text": r.get::<String, _>("source_text"),
            "target_text": r.get::<Option<String>, _>("target_text"),
            "state": r.get::<String, _>("state"),
            "memory_hit": r.get::<Option<String>, _>("memory_hit"),
        })
    }).collect())
}

pub async fn units_for_job(db: &Database, job_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => units_for_job_sqlite(db.sqlite_pool().expect("sqlite"), job_id).await,
        Backend::Postgres => units_for_job_postgres(db.postgres_pool().expect("postgres"), job_id).await,
    }
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

pub async fn add_memory(
    db: &Database,
    owner: &str,
    source_lang: &str,
    target_lang: &str,
    source_hash: &str,
    source_text: &str,
    target_text: &str,
    quality_bp: i64,
    shared: bool,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_memory (id, owner, source_lang, target_lang, source_hash, source_text, target_text, quality_bp, shared, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(owner).bind(source_lang).bind(target_lang).bind(source_hash).bind(source_text).bind(target_text).bind(quality_bp).bind(shared).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_memory (id, owner, source_lang, target_lang, source_hash, source_text, target_text, quality_bp, shared, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
            )
            .bind(&id).bind(owner).bind(source_lang).bind(target_lang).bind(source_hash).bind(source_text).bind(target_text).bind(quality_bp).bind(shared).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

async fn lookup_memory_sqlite(pool: &sqlx::SqlitePool, owner: &str, source_lang: &str, target_lang: &str, source_hash: &str) -> Result<Option<(String, i64, bool)>, sqlx::Error> {
    let row = sqlx::query("SELECT target_text, quality_bp, shared FROM translation_memory WHERE owner = ? AND source_lang = ? AND target_lang = ? AND source_hash = ?")
        .bind(owner).bind(source_lang).bind(target_lang).bind(source_hash)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get::<String, _>("target_text"), r.get::<i64, _>("quality_bp"), r.get::<bool, _>("shared"))))
}

async fn lookup_memory_postgres(pool: &sqlx::postgres::PgPool, owner: &str, source_lang: &str, target_lang: &str, source_hash: &str) -> Result<Option<(String, i64, bool)>, sqlx::Error> {
    let row = sqlx::query("SELECT target_text, quality_bp, shared FROM translation_memory WHERE owner = $1 AND source_lang = $2 AND target_lang = $3 AND source_hash = $4")
        .bind(owner).bind(source_lang).bind(target_lang).bind(source_hash)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| (r.get::<String, _>("target_text"), r.get::<i64, _>("quality_bp"), r.get::<bool, _>("shared"))))
}

pub async fn lookup_memory(
    db: &Database,
    owner: &str,
    source_lang: &str,
    target_lang: &str,
    source_hash: &str,
) -> Result<Option<(String, i64, bool)>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => lookup_memory_sqlite(db.sqlite_pool().expect("sqlite"), owner, source_lang, target_lang, source_hash).await,
        Backend::Postgres => lookup_memory_postgres(db.postgres_pool().expect("postgres"), owner, source_lang, target_lang, source_hash).await,
    }
}

// ---------------------------------------------------------------------------
// Glossaries
// ---------------------------------------------------------------------------

pub async fn add_glossary_term(
    db: &Database,
    owner: &str,
    work_id: Option<&str>,
    source_lang: &str,
    target_lang: &str,
    term: &str,
    translation: &str,
    case_sensitive: bool,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_glossaries (id, owner, work_id, source_lang, target_lang, term, translation, case_sensitive, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(owner).bind(work_id).bind(source_lang).bind(target_lang).bind(term).bind(translation).bind(case_sensitive).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_glossaries (id, owner, work_id, source_lang, target_lang, term, translation, case_sensitive, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
            )
            .bind(&id).bind(owner).bind(work_id).bind(source_lang).bind(target_lang).bind(term).bind(translation).bind(case_sensitive).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// Reviews
// ---------------------------------------------------------------------------

pub async fn open_review_gate(
    db: &Database,
    job_id: &str,
    reviewer: &str,
    gate: &ReviewGate,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_reviews (id, job_id, reviewer, gate, state, created_at)
                 VALUES (?, ?, ?, ?, 'pending', ?)"
            )
            .bind(&id).bind(job_id).bind(reviewer).bind(gate.as_str()).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_reviews (id, job_id, reviewer, gate, state, created_at)
                 VALUES ($1, $2, $3, $4, 'pending', $5)"
            )
            .bind(&id).bind(job_id).bind(reviewer).bind(gate.as_str()).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn decide_review_gate(
    db: &Database,
    review_id: &str,
    state: &str,
    notes: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE translation_reviews SET state = ?, notes = ?, decided_at = ? WHERE id = ?")
                .bind(state).bind(notes).bind(&now).bind(review_id)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE translation_reviews SET state = $1, notes = $2, decided_at = $3 WHERE id = $4")
                .bind(state).bind(notes).bind(&now).bind(review_id)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Publications
// ---------------------------------------------------------------------------

pub async fn record_publication(
    db: &Database,
    job_id: &str,
    work_id: &str,
) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO translation_publications (id, job_id, work_id, published_at)
                 VALUES (?, ?, ?, ?)"
            )
            .bind(&id).bind(job_id).bind(work_id).bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO translation_publications (id, job_id, work_id, published_at)
                 VALUES ($1, $2, $3, $4)"
            )
            .bind(&id).bind(job_id).bind(work_id).bind(&now)
            .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}
