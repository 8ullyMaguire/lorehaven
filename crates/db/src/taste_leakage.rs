//! §52.2 and §52.3 — the store behind the taste-leakage rules.
//!
//! Migration 0110 already refuses the forbidden shapes at the database level, so
//! most of what this module does is *report* rather than prevent. That is the
//! right division: a rule enforced in two places drifts, and the place that
//! cannot be argued with is the schema.
//!
//! What the store adds is what SQL cannot say — that a payout belongs to a batch
//! window, that a label is stale when its window has not closed, and the two
//! reads §52 needs.
//!
//! Every function is written in the shape `generated_content.rs` uses: a
//! `db.sql(...)` pair of templates, then a `match` whose arms each *bind* their
//! result to a typed local. The arms cannot yield their results instead, because
//! a `sqlx::Query<Sqlite>` and a `sqlx::Query<Postgres>` are different types — the
//! same reason the two templates cannot be one.

use lorehaven_domain::ids::{PseudId, WorkId};
use lorehaven_domain::leakage::{BatchWindow, BatchedPayout, Disposition, OwnerResonance};
use uuid::Uuid;

use crate::{Backend, Database, Result};

/// Parse a stored RFC 3339 timestamp into unix seconds.
///
/// The schema stores timestamps as text on both engines (0001_identity.sql sets
/// the convention), and the domain works in unix seconds. This is the one place
/// that converts, so a column declared TEXT cannot be read back as an integer.
fn fmt_ts(at: i64) -> String {
    // `identity::format_rfc3339` rather than a second `format` call, because its
    // doc comment says why it exists: a second formatter is a second thing that can
    // disagree about the offset, and the mismatch shows up as a timestamp that
    // parses into the wrong hour with nothing to say so.
    crate::identity::format_rfc3339(
        time::OffsetDateTime::from_unix_timestamp(at).expect("a timestamp in range"),
    )
}

fn parse_ts(s: &str) -> i64 {
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .expect("a stored timestamp is RFC 3339")
        .unix_timestamp()
}

/// A batch window's identity.
///
/// Not a domain id: a window is an operator-facing interval, and nothing outside
/// this module holds a typed handle to one.
pub type WindowId = Uuid;

/// Open a batch window.
///
/// §52.2's "at most one open window" is enforced by the schema (a trigger on
/// SQLite, a partial unique index on PostgreSQL). This returns the error rather
/// than papering over it, because an operator who opens a second window has a
/// real configuration problem and should be told which rule it hit.
pub async fn open_window(db: &Database, opened_at: i64) -> Result<WindowId> {
    let id = Uuid::new_v4();
    let sql = db.sql(
        "INSERT INTO taste_leakage_batch_windows (id, opened_at, closed_at) VALUES (?, ?, NULL)",
        "INSERT INTO taste_leakage_batch_windows (id, opened_at, closed_at) \
         VALUES ($1::uuid, $2, NULL)",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(id.to_string())
                .bind(fmt_ts(opened_at))
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(id)
                .bind(fmt_ts(opened_at))
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }
    Ok(id)
}

/// Close a batch window.
///
/// Idempotence is deliberately NOT provided. 0110 refuses re-closing, because a
/// payout or label inside a window is attributed to the interval the window
/// describes, and editing that interval retroactively widens what every row in
/// it claims to represent.
pub async fn close_window(db: &Database, id: WindowId, closed_at: i64) -> Result<()> {
    let sql = db.sql(
        "UPDATE taste_leakage_batch_windows SET closed_at = ? WHERE id = ?",
        "UPDATE taste_leakage_batch_windows SET closed_at = $1 WHERE id = $2::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(fmt_ts(closed_at))
                .bind(id.to_string())
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(fmt_ts(closed_at))
                .bind(id)
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }
    Ok(())
}

/// Read a window's bounds.
pub async fn get_window(db: &Database, id: WindowId) -> Result<Option<BatchWindow>> {
    let sql = db.sql(
        "SELECT opened_at, closed_at FROM taste_leakage_batch_windows WHERE id = ?",
        "SELECT opened_at, closed_at FROM taste_leakage_batch_windows WHERE id = $1::uuid",
    );
    // `opened_at`/`closed_at` are RFC 3339 TEXT columns on both engines, so they
    // decode as `String` and are parsed here. NOT as i64: that is the INT4/INT8
    // trap from `generated_content.rs`, wearing a different hat -- sqlx reports it
    // as "Rust type i64 (as SQL type INTEGER) is not compatible with SQL type
    // TEXT", and the two engines store the same column as TEXT.
    let stored: Option<(String, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(id)
                .fetch_optional(db.postgres_pool().expect("postgres pool"))
                .await?
        }
    };
    Ok(stored.map(|(opened_at, closed_at)| BatchWindow {
        opened_at: parse_ts(&opened_at),
        closed_at: closed_at.as_deref().map(parse_ts),
    }))
}

/// The most recent window, open or closed.
pub async fn latest_window(db: &Database) -> Result<Option<(WindowId, BatchWindow)>> {
    let sql = db.sql(
        "SELECT id, opened_at, closed_at FROM taste_leakage_batch_windows \
         ORDER BY opened_at DESC LIMIT 1",
        "SELECT id, opened_at, closed_at FROM taste_leakage_batch_windows \
         ORDER BY opened_at DESC LIMIT 1",
    );
    // `id::text` on PostgreSQL for the same reason as in `get_payout`: a native
    // UUID column does not decode into `String`.
    let stored: Option<(String, String, Option<String>)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(
                "SELECT id::text, opened_at, closed_at FROM taste_leakage_batch_windows \
                 ORDER BY opened_at DESC LIMIT 1",
            )
            .fetch_optional(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };
    Ok(stored.map(|(id, opened_at, closed_at)| {
        (
            Uuid::parse_str(&id).expect("a stored window id is a uuid"),
            BatchWindow {
                opened_at: parse_ts(&opened_at),
                closed_at: closed_at.as_deref().map(parse_ts),
            },
        )
    }))
}

/// Record a payout, attributed to a closed window.
///
/// There is deliberately no `attribution` parameter. 0110's CHECK admits only the
/// `instance` form, so the store writes that string itself and a caller cannot ask
/// for the forbidden one. §52.2's rule lives in the schema because a parameter
/// here would make it a convention.
pub async fn record_payout(
    db: &Database,
    recipient: PseudId,
    work: WorkId,
    credits: i64,
    window: WindowId,
    paid_at: i64,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let sql = db.sql(
        "INSERT INTO taste_leakage_payouts
            (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
         VALUES (?, ?, ?, ?, ?, 'instance', ?)",
        "INSERT INTO taste_leakage_payouts
            (id, recipient_pseud_id, work_id, credits, window_id, attribution, paid_at)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5::uuid, 'instance', $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(id.to_string())
                .bind(recipient.to_canonical_string())
                .bind(work.to_canonical_string())
                .bind(credits)
                .bind(window.to_string())
                .bind(fmt_ts(paid_at))
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(id)
                .bind(recipient.as_uuid())
                .bind(work.as_uuid())
                .bind(credits)
                .bind(window)
                .bind(fmt_ts(paid_at))
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }
    Ok(id)
}

/// Read a payout back, as §52.2 describes it.
///
/// The window's BOUNDS come back rather than its identity, because §52.2's rule
/// is about the interval and not about which window object happened to carry it.
pub async fn get_payout(db: &Database, id: Uuid) -> Result<Option<BatchedPayout>> {
    let sql = db.sql(
        "SELECT recipient_pseud_id, work_id, credits, opened_at, closed_at, paid_at
           FROM taste_leakage_payouts p
           JOIN taste_leakage_batch_windows w ON w.id = p.window_id
          WHERE p.id = ?",
        "SELECT recipient_pseud_id, work_id, credits, opened_at, closed_at, paid_at
           FROM taste_leakage_payouts p
           JOIN taste_leakage_batch_windows w ON w.id = p.window_id
          WHERE p.id = $1::uuid",
    );
    // Per-dialect decode for the id columns, and the reason is the same one that
    // forces two SQL templates: `pseuds.id` and `works.id` are TEXT on SQLite and
    // native UUID on PostgreSQL (0001_identity.sql), and sqlx checks the column
    // type against the Rust type exactly. A shared `String` arm therefore passes
    // every SQLite test and fails on the first PostgreSQL row with "Rust type String
    // (as SQL type TEXT) is not compatible with SQL type UUID".
    //
    // Timestamps, by contrast, are TEXT on BOTH engines, so those stay `String`
    // in both arms and go through `parse_ts`.
    #[allow(clippy::type_complexity)]
    let stored: Option<(String, String, i64, String, Option<String>, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(id.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            // Cast the UUID columns to text in the query itself rather than
            // decoding into a second set of Rust types, so the mapping below is
            // written once instead of once per engine.
            sqlx::query_as(
                "SELECT recipient_pseud_id::text, work_id::text, credits, \
                            opened_at, closed_at, paid_at \
                       FROM taste_leakage_payouts p \
                       JOIN taste_leakage_batch_windows w ON w.id = p.window_id \
                      WHERE p.id = $1::uuid",
            )
            .bind(id)
            .fetch_optional(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };
    Ok(stored.map(
        |(recipient, work, credits, opened_at, closed_at, paid_at)| BatchedPayout {
            recipient: PseudId::from_uuid(Uuid::parse_str(&recipient).expect("a stored uuid")),
            work: WorkId::from_uuid(Uuid::parse_str(&work).expect("a stored uuid")),
            credits,
            window: BatchWindow {
                opened_at: parse_ts(&opened_at),
                closed_at: closed_at.as_deref().map(parse_ts),
            },
            paid_at: parse_ts(&paid_at),
        },
    ))
}

/// Write the owner-visible resonance label for a work, from a batch.
///
/// Stored coarse — the four words, never a score — so the column holds no
/// precision to leak and there is nothing to round.
pub async fn set_resonance_label(
    db: &Database,
    owner: PseudId,
    work: WorkId,
    label: OwnerResonance,
    window: WindowId,
    computed_at: i64,
) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO taste_leakage_resonance_labels
            (work_id, owner_pseud_id, label, computed_at, batch_window_id)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (work_id) DO UPDATE SET
             label = EXCLUDED.label,
             computed_at = EXCLUDED.computed_at,
             batch_window_id = EXCLUDED.batch_window_id",
        "INSERT INTO taste_leakage_resonance_labels
            (work_id, owner_pseud_id, label, computed_at, batch_window_id)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5::uuid)
         ON CONFLICT (work_id) DO UPDATE SET
             label = EXCLUDED.label,
             computed_at = EXCLUDED.computed_at,
             batch_window_id = EXCLUDED.batch_window_id",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(work.to_canonical_string())
                .bind(owner.to_canonical_string())
                .bind(label.as_str())
                .bind(fmt_ts(computed_at))
                .bind(window.to_string())
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(work.as_uuid())
                .bind(owner.as_uuid())
                .bind(label.as_str())
                .bind(fmt_ts(computed_at))
                .bind(window)
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }
    Ok(())
}

/// Read a work's owner-visible label, if it has one.
pub async fn get_resonance_label(
    db: &Database,
    work: WorkId,
) -> Result<Option<(OwnerResonance, i64, WindowId)>> {
    let sql = db.sql(
        "SELECT label, computed_at, batch_window_id \
         FROM taste_leakage_resonance_labels WHERE work_id = ?",
        "SELECT label, computed_at, batch_window_id \
         FROM taste_leakage_resonance_labels WHERE work_id = $1::uuid",
    );
    // `batch_window_id::text` on PostgreSQL: native UUID, same reason as above.
    let stored: Option<(String, String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_canonical_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(
                "SELECT label, computed_at, batch_window_id::text \
                 FROM taste_leakage_resonance_labels WHERE work_id = $1::uuid",
            )
            .bind(work.as_uuid())
            .fetch_optional(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };
    Ok(stored.map(|(label, computed_at, window_id)| {
        (
            OwnerResonance::parse(&label).unwrap_or_else(|| {
                panic!("a stored label is one of the four, enforced by CHECK: {label}")
            }),
            parse_ts(&computed_at),
            Uuid::parse_str(&window_id).expect("a stored window id is a uuid"),
        )
    }))
}

/// Record a reviewed leakage row.
///
/// §52.1's default disposition is `keep`, applied as the column's DEFAULT rather
/// than here: a review inserted without saying what to do is a review that found
/// nothing wrong, and the store must not be the place that guesses otherwise.
pub async fn record_review(
    db: &Database,
    artifact: &str,
    inferable: &str,
    ease: &str,
    disposition: Disposition,
    reviewed_by: PseudId,
    reviewed_at: i64,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let sql = db.sql(
        "INSERT INTO taste_leakage_reviews
            (id, artifact, inferable, ease, disposition, reviewed_by, reviewed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        "INSERT INTO taste_leakage_reviews
            (id, artifact, inferable, ease, disposition, reviewed_by, reviewed_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6::uuid, $7)",
    );
    match db.backend() {
        Backend::Sqlite => {
            let _ = sqlx::query(&sql)
                .bind(id.to_string())
                .bind(artifact)
                .bind(inferable)
                .bind(ease)
                .bind(disposition.as_str())
                .bind(reviewed_by.to_canonical_string())
                .bind(fmt_ts(reviewed_at))
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            let _ = sqlx::query(&sql)
                .bind(id)
                .bind(artifact)
                .bind(inferable)
                .bind(ease)
                .bind(disposition.as_str())
                .bind(reviewed_by.as_uuid())
                .bind(fmt_ts(reviewed_at))
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }
    Ok(id)
}

/// Read reviewed rows, optionally narrowed to one disposition.
///
/// No LIMIT and no COUNT. §52.1: a view that reports how many artifacts it
/// reviewed is itself a probe, because the difference between the count now and
/// after a configuration change is a measurement of the operator's taste. The
/// `Disposition` filter exists for the operator's own convenience and carries the
/// same risk as the count, so it is opt-in per request rather than a default the
/// caller has to remember to suppress.
pub async fn review_rows(
    db: &Database,
    disposition: Option<Disposition>,
) -> Result<Vec<(String, String, String, Disposition, PseudId, i64)>> {
    // Two statements rather than one with a nullable predicate: a shared template
    // cannot both filter and not filter, and `($5 IS NULL OR disposition = $5)`
    // is the kind of cleverness that reads fine and plans badly.
    // `reviewed_by::text` on PostgreSQL. `pseuds.id` is TEXT on SQLite and native
    // UUID there, and the returned rows decode into `String`, so without the cast
    // every PostgreSQL request 500s on "Rust type String (as SQL type TEXT) is not
    // compatible with SQL type UUID" -- while every SQLite test passes. The cast is
    // in the SQL rather than in a second set of Rust types so the mapping below is
    // written once.
    let base = match db.backend() {
        Backend::Sqlite => {
            "SELECT artifact, inferable, ease, disposition, reviewed_by, reviewed_at \
             FROM taste_leakage_reviews"
        }
        Backend::Postgres => {
            "SELECT artifact, inferable, ease, disposition, reviewed_by::text, reviewed_at \
             FROM taste_leakage_reviews"
        }
    };
    // Bound to locals first: `db.sql` returns a borrow, and a `format!` temporary
    // passed straight into it is dropped at the end of the statement.
    let all_sql = format!("{base} ORDER BY reviewed_at DESC");
    let filtered_sql = format!("{base} WHERE disposition = {{}} ORDER BY reviewed_at DESC");
    let filtered_sqlite = filtered_sql.replace("{}", "?");
    let filtered_postgres = filtered_sql.replace("{}", "$1");
    let sql = match disposition {
        None => db.sql(&all_sql, &all_sql),
        Some(_) => db.sql(&filtered_sqlite, &filtered_postgres),
    };
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, String)> =
        match (db.backend(), disposition) {
            (Backend::Sqlite, None) => {
                sqlx::query_as(&sql)
                    .fetch_all(db.sqlite_pool().expect("sqlite pool"))
                    .await?
            }
            (Backend::Sqlite, Some(d)) => {
                sqlx::query_as(&sql)
                    .bind(d.as_str())
                    .fetch_all(db.sqlite_pool().expect("sqlite pool"))
                    .await?
            }
            (Backend::Postgres, None) => {
                sqlx::query_as(&sql)
                    .fetch_all(db.postgres_pool().expect("postgres pool"))
                    .await?
            }
            (Backend::Postgres, Some(d)) => {
                sqlx::query_as(&sql)
                    .bind(d.as_str())
                    .fetch_all(db.postgres_pool().expect("postgres pool"))
                    .await?
            }
        };
    Ok(rows
        .into_iter()
        .map(
            |(artifact, inferable, ease, disposition, reviewed_by, reviewed_at)| {
                (
                    artifact,
                    inferable,
                    ease,
                    Disposition::parse(&disposition).unwrap_or_else(|| {
                        panic!(
                            "a stored disposition is one of the three, enforced by CHECK: \
                         {disposition}"
                        )
                    }),
                    PseudId::from_uuid(Uuid::parse_str(&reviewed_by).expect("a stored uuid")),
                    parse_ts(&reviewed_at),
                )
            },
        )
        .collect())
}
