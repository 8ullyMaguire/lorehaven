//! M45 — Roadmap consensus: cards, ballots, moves (spec §44).
//!
//! Dual-backend pattern: SQLite + Postgres via `match db.backend()`.

use sqlx::Row;

use crate::{Backend, Database};

/// A feature card on the public roadmap board.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub category: String,
    pub stage: String,
    pub elo_rating: f64,
    pub matches_played: i64,
    pub times_best: i64,
    pub times_worst: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Upsert a card: insert new or update existing by id.
///
/// §44.6: never downgrades a `shipped` or `rejected` card's stage.
pub async fn upsert_card(db: &Database, card: &Card) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            // Protect shipped/rejected from stage downgrade.
            sqlx::query(
                "INSERT INTO roadmap_cards (id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                   title=excluded.title,
                   category=excluded.category,
                   stage=excluded.stage,
                   updated_at=excluded.updated_at
                 WHERE stage NOT IN ('shipped','rejected')",
            )
            .bind(&card.id)
            .bind(&card.title)
            .bind(&card.category)
            .bind(&card.stage)
            .bind(card.elo_rating)
            .bind(card.matches_played)
            .bind(card.times_best)
            .bind(card.times_worst)
            .bind(&now)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO roadmap_cards (id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(id) DO UPDATE SET
                   title=excluded.title,
                   category=excluded.category,
                   stage=excluded.stage,
                   updated_at=excluded.updated_at
                 WHERE roadmap_cards.stage NOT IN ('shipped','rejected')",
            )
            .bind(&card.id)
            .bind(&card.title)
            .bind(&card.category)
            .bind(&card.stage)
            .bind(card.elo_rating)
            .bind(card.matches_played)
            .bind(card.times_best)
            .bind(card.times_worst)
            .bind(&now)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Normalize a title for deduplication: lowercase, punctuation stripped, ws collapsed.
fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let stripped: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let mut result = String::new();
    let mut prev_space = false;
    for c in stripped.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Find a card by normalized title. Returns None if no match.
pub async fn find_card_by_title_normalized(
    db: &Database,
    title: &str,
) -> Result<Option<Card>, sqlx::Error> {
    let target = normalize_title(title);
    let cards = list_cards(db, None).await?;
    Ok(cards
        .into_iter()
        .find(|c| normalize_title(&c.title) == target))
}

/// List cards, optionally filtered by stage. Elo DESC, tie-break matches_played DESC, card_id ASC.
pub async fn list_cards(db: &Database, stage: Option<&str>) -> Result<Vec<Card>, sqlx::Error> {
    let sql = match stage {
        Some(_) => "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards WHERE stage = ? ORDER BY elo_rating DESC, matches_played DESC, id ASC",
        None => "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards ORDER BY elo_rating DESC, matches_played DESC, id ASC",
    };
    let sql_pg = match stage {
        Some(_) => "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards WHERE stage = $1 ORDER BY elo_rating DESC, matches_played DESC, id ASC",
        None => "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards ORDER BY elo_rating DESC, matches_played DESC, id ASC",
    };
    match db.backend() {
        Backend::Sqlite => {
            let query = sqlx::query(sql);
            let query = match stage {
                Some(s) => query.bind(s),
                None => query,
            };
            let rows = query.fetch_all(db.sqlite_pool().expect("sqlite")).await?;
            Ok(rows.iter().map(row_to_card_sqlite).collect())
        }
        Backend::Postgres => {
            let query = sqlx::query(sql_pg);
            let query = match stage {
                Some(s) => query.bind(s),
                None => query,
            };
            let rows = query
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows.iter().map(row_to_card_postgres).collect())
        }
    }
}

/// Pick N random `idea`-stage cards for an arena ballot.
pub async fn arena_candidates(db: &Database, limit: i64) -> Result<Vec<Card>, sqlx::Error> {
    let sql = "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards WHERE stage = 'idea' ORDER BY RANDOM() LIMIT ?";
    let sql_pg = "SELECT id, title, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards WHERE stage = 'idea' ORDER BY RANDOM() LIMIT $1";
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(sql)
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows.iter().map(row_to_card_sqlite).collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query(sql_pg)
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows.iter().map(row_to_card_postgres).collect())
        }
    }
}

/// Create a ballot of 4 cards with their pre-match Elo.
pub async fn create_ballot(
    db: &Database,
    ballot_id: &str,
    card_ids: &[String],
    served_elo: &[(String, f64)],
    account_id: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let card_ids_json = serde_json::to_string(card_ids).expect("json");
    let served_elo_json = serde_json::to_string(served_elo).expect("json");
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO roadmap_ballots (id, card_ids, served_elo, account_id, created_at, voted_at)
                 VALUES (?, ?, ?, ?, ?, NULL)",
            )
            .bind(ballot_id)
            .bind(&card_ids_json)
            .bind(&served_elo_json)
            .bind(account_id)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO roadmap_ballots (id, card_ids, served_elo, account_id, created_at, voted_at)
                 VALUES ($1, $2::jsonb, $3::jsonb, $4, $5, NULL)",
            )
            .bind(ballot_id)
            .bind(&card_ids_json)
            .bind(&served_elo_json)
            .bind(account_id)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Fetch a ballot by id.
pub async fn fetch_ballot(
    db: &Database,
    ballot_id: &str,
) -> Result<Option<(Vec<String>, Vec<(String, f64)>, Option<String>)>, sqlx::Error> {
    let sql = "SELECT card_ids, served_elo, voted_at FROM roadmap_ballots WHERE id = ?";
    let sql_pg = "SELECT card_ids, served_elo, voted_at FROM roadmap_ballots WHERE id = $1";
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(sql)
                .bind(ballot_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.map(|r| {
                let card_ids_json: String = r.get(0);
                let served_elo_json: String = r.get(1);
                let voted_at: Option<String> = r.get(2);
                let card_ids: Vec<String> = serde_json::from_str(&card_ids_json).expect("json");
                let served_elo: Vec<(String, f64)> =
                    serde_json::from_str(&served_elo_json).expect("json");
                (card_ids, served_elo, voted_at)
            }))
        }
        Backend::Postgres => {
            let row = sqlx::query(sql_pg)
                .bind(ballot_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.map(|r| {
                let card_ids_json: String = r.get(0);
                let served_elo_json: String = r.get(1);
                let voted_at: Option<String> = r.get(2);
                let card_ids: Vec<String> = serde_json::from_str(&card_ids_json).expect("json");
                let served_elo: Vec<(String, f64)> =
                    serde_json::from_str(&served_elo_json).expect("json");
                (card_ids, served_elo, voted_at)
            }))
        }
    }
}

/// Mark a ballot as voted (atomic one-vote-per-ballot). Returns true if the vote was recorded.
pub async fn mark_voted(db: &Database, ballot_id: &str) -> Result<bool, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let result = sqlx::query(
                "UPDATE roadmap_ballots SET voted_at = ? WHERE id = ? AND voted_at IS NULL",
            )
            .bind(&now)
            .bind(ballot_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
            Ok(result.rows_affected() > 0)
        }
        Backend::Postgres => {
            let result = sqlx::query(
                "UPDATE roadmap_ballots SET voted_at = $1 WHERE id = $2 AND voted_at IS NULL",
            )
            .bind(&now)
            .bind(ballot_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
            Ok(result.rows_affected() > 0)
        }
    }
}

/// Apply Elo updates and counters after a vote.
pub async fn apply_elo_and_counters(
    db: &Database,
    updates: &[(String, f64, bool, bool)],
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            for (card_id, new_elo, is_best, is_worst) in updates {
                sqlx::query(
                    "UPDATE roadmap_cards SET elo_rating = ?, matches_played = matches_played + 1,
                     times_best = times_best + ?, times_worst = times_worst + ?, updated_at = ? WHERE id = ?",
                )
                .bind(new_elo)
                .bind(if *is_best { 1 } else { 0 })
                .bind(if *is_worst { 1 } else { 0 })
                .bind(&now)
                .bind(card_id)
                .execute(pool)
                .await?;
            }
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            for (card_id, new_elo, is_best, is_worst) in updates {
                sqlx::query(
                    "UPDATE roadmap_cards SET elo_rating = $1, matches_played = matches_played + 1,
                     times_best = times_best + $2, times_worst = times_worst + $3, updated_at = $4 WHERE id = $5",
                )
                .bind(new_elo)
                .bind(if *is_best { 1 } else { 0 })
                .bind(if *is_worst { 1 } else { 0 })
                .bind(&now)
                .bind(card_id)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}

/// Record a stage move in the changelog.
pub async fn record_move(
    db: &Database,
    card_id: &str,
    from_stage: &str,
    to_stage: &str,
    reason: &str,
    moved_by: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO roadmap_moves (id, card_id, from_stage, to_stage, reason, moved_by, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(card_id)
            .bind(from_stage)
            .bind(to_stage)
            .bind(reason)
            .bind(moved_by)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO roadmap_moves (id, card_id, from_stage, to_stage, reason, moved_by, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&id)
            .bind(card_id)
            .bind(from_stage)
            .bind(to_stage)
            .bind(reason)
            .bind(moved_by)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// List recent stage moves, newest first, paginated.
pub async fn list_moves(
    db: &Database,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, String, String, String)>, sqlx::Error> {
    let sql = "SELECT card_id, from_stage, to_stage, reason, moved_by, created_at FROM roadmap_moves ORDER BY created_at DESC LIMIT ? OFFSET ?";
    let sql_pg = "SELECT card_id, from_stage, to_stage, reason, moved_by, created_at FROM roadmap_moves ORDER BY created_at DESC LIMIT $1 OFFSET $2";
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    (
                        r.get::<String, _>(0),
                        r.get::<String, _>(1),
                        r.get::<String, _>(2),
                        r.get::<String, _>(3),
                        r.get::<String, _>(4),
                        r.get::<String, _>(5),
                    )
                })
                .collect())
        }
        Backend::Postgres => {
            let rows = sqlx::query(sql_pg)
                .bind(limit)
                .bind(offset)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(rows
                .iter()
                .map(|r| {
                    (
                        r.get::<String, _>(0),
                        r.get::<String, _>(1),
                        r.get::<String, _>(2),
                        r.get::<String, _>(3),
                        r.get::<String, _>(4),
                        r.get::<String, _>(5),
                    )
                })
                .collect())
        }
    }
}

/// Insert a suggestion. If `card_id` is provided, attach to that card.
pub async fn insert_suggestion(
    db: &Database,
    account_id: &str,
    raw_text: &str,
    card_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO roadmap_suggestions (id, account_id, raw_text, card_id, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(account_id)
            .bind(raw_text)
            .bind(card_id)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO roadmap_suggestions (id, account_id, raw_text, card_id, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&id)
            .bind(account_id)
            .bind(raw_text)
            .bind(card_id)
            .bind(&now)
            .execute(db.postgres_pool().expect("postgres"))
            .await?;
        }
    }
    Ok(())
}

/// Update a card's stage (operator-only).
pub async fn update_card_stage(
    db: &Database,
    card_id: &str,
    stage: &str,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE roadmap_cards SET stage = ?, updated_at = ? WHERE id = ?")
                .bind(stage)
                .bind(&now)
                .bind(card_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE roadmap_cards SET stage = $1, updated_at = $2 WHERE id = $3")
                .bind(stage)
                .bind(&now)
                .bind(card_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

// --- Row mappers ---

fn row_to_card_sqlite(row: &sqlx::sqlite::SqliteRow) -> Card {
    Card {
        id: row.get::<String, _>(0),
        title: row.get::<String, _>(1),
        category: row.get::<String, _>(2),
        stage: row.get::<String, _>(3),
        elo_rating: row.get::<f64, _>(4),
        matches_played: row.get::<i64, _>(5),
        times_best: row.get::<i64, _>(6),
        times_worst: row.get::<i64, _>(7),
        created_at: row.get::<String, _>(8),
        updated_at: row.get::<String, _>(9),
    }
}

fn row_to_card_postgres(row: &sqlx::postgres::PgRow) -> Card {
    Card {
        id: row.get::<String, _>(0),
        title: row.get::<String, _>(1),
        category: row.get::<String, _>(2),
        stage: row.get::<String, _>(3),
        elo_rating: row.get::<f64, _>(4),
        matches_played: row.get::<i32, _>(5) as i64,
        times_best: row.get::<i32, _>(6) as i64,
        times_worst: row.get::<i32, _>(7) as i64,
        created_at: row.get::<String, _>(8),
        updated_at: row.get::<String, _>(9),
    }
}
