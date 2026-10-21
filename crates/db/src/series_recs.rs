//! §16.1 — series-aware recommendation: the next unread entry in a series.
//!
//! The gap this closes is #21 on the ideas list, described there as "the cheapest
//! real engine gap — the data is already there and the recommendation is a query
//! over it". The data was indeed already there (`media_collections` with
//! `collection_kind = 'series'`, and `media_collection_items.position`), and it was
//! indeed unreachable: `media::collection_media` returns *media* records, not the
//! works in a collection, so nothing could walk a series in order.
//!
//! Five decisions, each of which produces a wrong-but-plausible answer if ignored.
//!
//! 1. **The suggestion is the first UNFINISHED position, not the one after your
//!    furthest finished entry.** These differ the moment a reader skips, which is
//!    most readers. Someone who has read entries 1, 2 and 4 of a five-part series
//!    must be offered entry 3 — the gap is the thing they are missing, and an
//!    implementation anchoring on the furthest finished entry suggests 5 and never
//!    mentions 3, abandoning the gap silently. That was the second version of this
//!    query; `a_skipped_entry_is_still_the_suggestion` caught it.
//!
//! 2. **A finished work is never suggested, and an *abandoned* one still is.** That
//!    distinction is the whole feature: exclusion is "already finished", never
//!    "already started". A reader who abandoned entry 2 midway should still be
//!    offered entry 2 — suggesting entry 3 instead hides an unfinished book without
//!    telling anyone.
//!
//! 3. **The author's numbering is the only ordering that exists.** There is no
//!    "smart" gap-filling or popularity order, because no signal here could support
//!    one and inventing an order would silently contradict the author's.
//!
//! 4. **A one-entry series yields nothing.** With one entry there is no next, and
//!    returning that entry would recommend a finished work — the one outcome a feed
//!    must never produce.
//!
//! 5. **Cold start is its own origin, not a special case.** A reader who *owns* a
//!    series and has finished nothing in it is offered entry one, and [`Origin`] says
//!    which of the two situations produced a suggestion, so a reason line can tell a
//!    reader why.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Backend, Database, Result};

/// Which way to walk a series from what a reader has finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// The first unfinished entry: the "suggest the next unread entry in a series you
    /// finished" of §16.1, and the only direction implemented.
    Forward,
    /// The entry before their earliest finished one, for "what did I miss?".
    ///
    /// Declared because the question is real and will be asked, and **not wired to
    /// anything**: a backward suggestion competes with the forward one for the same
    /// slot in the feed, and choosing between them is a product decision rather than
    /// a database one. An enum rather than a boolean so adding the query later is not
    /// an API change.
    Backward,
}

/// Why a work was suggested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// The reader finished something in this series and this is the first entry they
    /// have not.
    NextAfterFinished,
    /// The reader owns this series and has finished nothing in it; this is entry one.
    FirstInSeries,
}

/// One series a reader has a reason to care about, and its next entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextEntry {
    pub collection_id: String,
    pub series_title: String,
    /// The nearest finished entry *before* this one, or `None` when the suggestion is
    /// a cold start.
    pub after_work_id: Option<String>,
    pub after_position: Option<i64>,
    /// The entry to read. Never one the reader has already finished.
    pub work_id: String,
    pub position: i64,
    pub entry_title: String,
    pub origin: Origin,
}

/// One row of the query, before it becomes a [`NextEntry`].
///
/// Declared at module scope rather than per dialect arm: two structurally identical
/// local types are still different types, and the arms would not unify.
#[derive(Debug, Clone, sqlx::FromRow)]
struct SeriesRow {
    collection_id: String,
    series_title: String,
    after_work_id: Option<String>,
    after_position: Option<i64>,
    work_id: String,
    position: i64,
    entry_title: String,
    origin: String,
}

/// The most suggestions one call returns.
///
/// One per series is the *intent*; this bounds the whole result so a data error
/// cannot turn a recommendation surface into a catalogue dump. The first version used
/// `LIMIT 1`, which capped the entire answer at one suggestion regardless of how many
/// series qualified — `two_series_contribute_one_suggestion_each` caught it.
const MAX_SUGGESTIONS: i64 = 50;

/// The next entries to read in every series the reader has finished something in,
/// plus every series they own and have read nothing in.
///
/// `since`/`until` are unix seconds bounding the window of *finishing* that qualifies
/// a series, matching `hit_rate`'s convention so a caller passes one kind of window
/// everywhere.
pub async fn next_entries(
    db: &Database,
    account_id: &str,
    since: i64,
    until: i64,
) -> Result<Vec<NextEntry>> {
    // **Five parameters, numbered $1..$5 on both dialects.** A first version numbered
    // the PostgreSQL arm from `$2`, because `account_id` appears three times in the
    // query and got its own slot -- and then never used `$1` at all. sqlx binds
    // POSITIONALLY, so `$1` silently received `since`, and the query compared an RFC
    // 3339 string against a bigint: "operator does not exist: text >= integer".
    //
    // Both arms now number from 1 and differ only in the marker, the `::text` casts
    // and how the window is compared. Writing them from one template rather than by
    // hand is what keeps them that way: the two were edited independently and drifted,
    // which is how the numbering hole appeared at all.
    let rows: Vec<SeriesRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, SeriesRow>(
                r#"
                WITH series_items AS (
                    SELECT c.id AS collection_id, c.title AS series_title,
                           i.work_id AS work_id, i.position AS position,
                           w.title AS entry_title
                    FROM media_collections c
                    JOIN media_collection_items i ON i.collection_id = c.id
                    JOIN works w ON w.id = i.work_id
                    WHERE c.collection_kind = 'series'
                ),
                finished AS (
                    SELECT si.collection_id, si.work_id, si.position
                    FROM series_items si
                    JOIN reading_status rs
                      ON rs.subject_type = 'work' AND rs.subject_id = si.work_id
                     AND rs.account_id = ?1 AND rs.status = 'finished'
                     AND rs.finished_at IS NOT NULL
                    WHERE CAST(strftime('%s', rs.finished_at) AS INTEGER) >= ?2
                      AND CAST(strftime('%s', rs.finished_at) AS INTEGER) <  ?3
                ),
                candidate AS (
                    -- The first UNFINISHED entry per series. An anti-join rather than
                    -- `MAX(finished) + 1`, so a gap in the reader's history is the
                    -- answer rather than something skipped over.
                    SELECT collection_id, MIN(position) AS position
                    FROM series_items si
                    WHERE NOT EXISTS (
                        SELECT 1 FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position = si.position)
                    GROUP BY collection_id
                )
                SELECT si.collection_id AS collection_id, si.series_title,
                       -- The nearest finished entry before this one, for the reason
                       -- line. Two correlated subqueries rather than a join, because
                       -- the answer is a *nearest* and a join would need LATERAL,
                       -- which SQLite lacks.
                       (SELECT f.work_id FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position < si.position
                         ORDER BY f.position DESC LIMIT 1) AS after_work_id,
                       (SELECT MAX(f.position) FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position < si.position) AS after_position,
                       si.work_id, si.position, si.entry_title,
                       'next_after_finished' AS origin
                FROM candidate c
                JOIN series_items si
                  ON si.collection_id = c.collection_id AND si.position = c.position
                -- A series with nothing finished has no "next after", only a cold
                -- start, and the UNION below may already have claimed it.
                WHERE EXISTS (
                    SELECT 1 FROM finished f WHERE f.collection_id = c.collection_id)
                UNION ALL
                SELECT c.id, c.title, NULL, NULL,
                       si.work_id, si.position, w.title, 'first_in_series'
                FROM media_collections c
                JOIN media_collection_items si ON si.collection_id = c.id
                JOIN works w ON w.id = si.work_id
                WHERE c.collection_kind = 'series'
                  AND c.owning_account_id = ?1
                  AND si.position = 1
                  AND NOT EXISTS (
                      SELECT 1 FROM reading_status rs
                       WHERE rs.subject_type = 'work'
                         AND rs.subject_id IN (
                             SELECT i2.work_id FROM media_collection_items i2
                              WHERE i2.collection_id = c.id)
                         AND rs.account_id = ?1 AND rs.status = 'finished')
                LIMIT ?4
                "#,
            )
            .bind(account_id)
            .bind(since)
            .bind(until)
            .bind(MAX_SUGGESTIONS)
            .fetch_all(db.sqlite_pool().expect("sqlite pool"))
            .await?
        }
        Backend::Postgres => {
            // Three things differ here, all from the schema rather than preference:
            //   * `reading_status.finished_at` is TEXT, so the window arrives as RFC
            //     3339 text and compares lexicographically -- correct for fixed-width
            //     UTC, and cheaper than casting.
            //   * ids are native uuid on this dialect, so every join against the text
            //     columns above spells `::text`.
            //   * `owning_account_id` is uuid, hence `::text` to match the bound.
            sqlx::query_as::<_, SeriesRow>(
                r#"
                WITH series_items AS (
                    SELECT c.id AS collection_id, c.title AS series_title,
                           i.work_id::text AS work_id, i.position AS position,
                           w.title AS entry_title
                    FROM media_collections c
                    JOIN media_collection_items i ON i.collection_id = c.id
                    JOIN works w ON w.id = i.work_id
                    WHERE c.collection_kind = 'series'
                ),
                finished AS (
                    SELECT si.collection_id, si.work_id, si.position
                    FROM series_items si
                    JOIN reading_status rs
                      ON rs.subject_type = 'work' AND rs.subject_id::text = si.work_id
                     AND rs.account_id::text = $1 AND rs.status = 'finished'
                     AND rs.finished_at IS NOT NULL
                    WHERE rs.finished_at >= $2 AND rs.finished_at < $3
                ),
                candidate AS (
                    -- The first UNFINISHED entry per series. An anti-join rather than
                    -- `MAX(finished) + 1`, so a gap in the reader's history is the
                    -- answer rather than something skipped over.
                    SELECT collection_id, MIN(position) AS position
                    FROM series_items si
                    WHERE NOT EXISTS (
                        SELECT 1 FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position = si.position)
                    GROUP BY collection_id
                )
                SELECT si.collection_id::text AS collection_id, si.series_title,
                       -- The nearest finished entry before this one, for the reason
                       -- line. Two correlated subqueries rather than a join, because
                       -- the answer is a *nearest* and a join would need LATERAL,
                       -- which SQLite lacks.
                       (SELECT f.work_id FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position < si.position
                         ORDER BY f.position DESC LIMIT 1) AS after_work_id,
                       (SELECT MAX(f.position) FROM finished f
                         WHERE f.collection_id = si.collection_id
                           AND f.position < si.position) AS after_position,
                       si.work_id, si.position, si.entry_title,
                       'next_after_finished'::text AS origin
                FROM candidate c
                JOIN series_items si
                  ON si.collection_id = c.collection_id AND si.position = c.position
                -- A series with nothing finished has no "next after", only a cold
                -- start, and the UNION below may already have claimed it.
                WHERE EXISTS (
                    SELECT 1 FROM finished f WHERE f.collection_id = c.collection_id)
                UNION ALL
                SELECT c.id::text, c.title, NULL, NULL,
                       si.work_id::text, si.position, w.title, 'first_in_series'::text
                FROM media_collections c
                JOIN media_collection_items si ON si.collection_id = c.id
                JOIN works w ON w.id = si.work_id
                WHERE c.collection_kind = 'series'
                  AND c.owning_account_id::text = $1
                  AND si.position = 1
                  AND NOT EXISTS (
                      SELECT 1 FROM reading_status rs
                       WHERE rs.subject_type = 'work'
                         AND rs.subject_id::text IN (
                             SELECT i2.work_id::text FROM media_collection_items i2
                              WHERE i2.collection_id = c.id)
                         AND rs.account_id::text = $1 AND rs.status = 'finished')
                LIMIT $4
                "#,
            )
            .bind(account_id)
            .bind(rfc3339(since))
            .bind(rfc3339(until))
            .bind(MAX_SUGGESTIONS)
            .fetch_all(db.postgres_pool().expect("postgres pool"))
            .await?
        }
    };

    Ok(rows.into_iter().filter_map(SeriesRow::into_next).collect())
}

/// Unix seconds to the RFC 3339 text `reading_status` stores on PostgreSQL.
fn rfc3339(unix: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix)
        .expect("a representable timestamp")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("an RFC 3339 string")
}

impl SeriesRow {
    /// Place the origin, dropping rows that name an unknown one.
    ///
    /// The origin is a `TEXT` literal the query produced, so an unrecognised value
    /// means this build and the query have drifted apart — worth an error rather
    /// than a silent default. A work id that will not parse is dropped silently: the
    /// alternative is a feed row pointing at nothing.
    fn into_next(self) -> Option<NextEntry> {
        Uuid::parse_str(&self.work_id).ok()?;
        let origin = match self.origin.as_str() {
            "next_after_finished" => Origin::NextAfterFinished,
            "first_in_series" => Origin::FirstInSeries,
            other => {
                tracing::error!(
                    origin = %other,
                    "a series row has an origin this build does not know; the query and \
                     the enum have drifted apart"
                );
                return None;
            }
        };
        Some(NextEntry {
            collection_id: self.collection_id,
            series_title: self.series_title,
            after_work_id: self.after_work_id,
            after_position: self.after_position,
            work_id: self.work_id,
            position: self.position,
            entry_title: self.entry_title,
            origin,
        })
    }
}

// ---------------------------------------------------------------------------
// Series, relations and the "more like this" ladder (spec §6.2, §11.5)
// ---------------------------------------------------------------------------

/// An explicit relation between two works (spec §6.2).
///
/// **A closed vocabulary of five**, and the same list the schema's `CHECK`
/// holds. A relation outside it is not a relation the instance can explain,
/// and a free-text relation would make "next in series" and "the next one,
/// probably" indistinguishable — which is exactly the distinction the ladder's
/// first rung rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRelation {
    /// The next work in the series. The ladder's first rung reads this one.
    NextInSeries,
    /// The work that came before this one.
    Prequel,
    /// The work that came after this one.
    Sequel,
    /// A story in the same world that is not part of the series.
    SideStory,
    /// A translation of this work.
    TranslationOf,
}

impl WorkRelation {
    /// The relation's stable name, as the schema stores it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NextInSeries => "next_in_series",
            Self::Prequel => "prequel",
            Self::Sequel => "sequel",
            Self::SideStory => "side_story",
            Self::TranslationOf => "translation_of",
        }
    }

    /// Parse the stored name, refusing an unknown relation.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "next_in_series" => Some(Self::NextInSeries),
            "prequel" => Some(Self::Prequel),
            "sequel" => Some(Self::Sequel),
            "side_story" => Some(Self::SideStory),
            "translation_of" => Some(Self::TranslationOf),
            _ => None,
        }
    }
}

/// One ladder recommendation, as the store produces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedWork {
    /// The work recommended.
    pub work_id: String,
    /// The reason, in the words the reader sees.
    pub reason: String,
}

/// The works explicitly related to this one as *next in series*, with the
/// series' title.
///
/// **The explicit relation, not the series' order** (spec §11.5). The series'
/// order is the author's, and a relation is a statement about two works that
/// may not be in a series at all — so the ladder's first rung reads the
/// relation and the series is where the *reason* comes from.
pub async fn next_in_series(db: &Database, work_id: &str) -> Result<Vec<RelatedWork>> {
    let sql = db.sql(
        "SELECT r.to_work_id, COALESCE(s.title, '')
           FROM work_relations r
           LEFT JOIN series_entries e ON e.work_id = r.from_work_id
           LEFT JOIN series s ON s.id = e.series_id
          WHERE r.from_work_id = ? AND r.relation_type = 'next_in_series'
          ORDER BY r.to_work_id",
        "SELECT r.to_work_id::text, COALESCE(s.title, '')
           FROM work_relations r
           LEFT JOIN series_entries e ON e.work_id = r.from_work_id
           LEFT JOIN series s ON s.id = e.series_id
          WHERE r.from_work_id = $1::uuid AND r.relation_type = 'next_in_series'
          ORDER BY r.to_work_id::text",
    );
    let rows: Vec<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(work_id, series_title)| RelatedWork {
            work_id,
            reason: if series_title.is_empty() {
                "the next one in its series".to_owned()
            } else {
                format!("the next one in {series_title}")
            },
        })
        .collect())
}

/// The same author's other published works, with the author's public name.
///
/// **The author's public face, never the account.** The name on a byline is
/// what a reader knows the author by, and the account behind it is not
/// something this surface may publish (spec §7).
pub async fn same_author_works(db: &Database, work_id: &str) -> Result<Vec<RelatedWork>> {
    let sql = db.sql(
        "SELECT w.id, COALESCE(p.display_name, p.handle)
           FROM works w
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE w.owner_pseud_id = (SELECT owner_pseud_id FROM works WHERE id = ?)
            AND w.id <> ?
            AND w.lifecycle = 'published'
            AND w.visibility = 'public'
            AND w.deleted_at IS NULL
          ORDER BY w.published_at DESC, w.id",
        "SELECT w.id::text, COALESCE(p.display_name, p.handle)
           FROM works w
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE w.owner_pseud_id = (SELECT owner_pseud_id FROM works WHERE id = $1::uuid)
            AND w.id <> $1::uuid
            AND w.lifecycle = 'published'
            AND w.visibility = 'public'
            AND w.deleted_at IS NULL
          ORDER BY w.published_at DESC, w.id::text",
    );
    let rows: Vec<(String, String)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(work_id, author_name)| RelatedWork {
            work_id,
            reason: format!("more by {author_name}"),
        })
        .collect())
}

/// Record an explicit relation between two works.
pub async fn relate_works(
    db: &Database,
    from_work_id: &str,
    to_work_id: &str,
    relation: WorkRelation,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let sql = db.sql(
        "INSERT INTO work_relations (id, from_work_id, to_work_id, relation_type, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, 1)",
        "INSERT INTO work_relations (id, from_work_id, to_work_id, relation_type, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, 1)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(from_work_id)
                .bind(to_work_id)
                .bind(relation.as_str())
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&id)
                .bind(from_work_id)
                .bind(to_work_id)
                .bind(relation.as_str())
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(id)
}

/// Put a work in a series, creating the series if it is new.
///
/// **The series is created on demand** rather than by a separate call, because
/// a series with one entry is the common case and a two-step dance to publish
/// one work in a series is a step nobody will take.
///
/// The position is the author's and there is no other order.
pub async fn put_in_series(
    db: &Database,
    slug: &str,
    title: &str,
    work_id: &str,
    position: i64,
    owner_pseud_id: Option<String>,
) -> Result<String, sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let series_id = uuid::Uuid::new_v4().to_string();

    // The series, upserted by its slug. `ON CONFLICT DO NOTHING` and then a
    // read, rather than an upsert that overwrites the title: a series' title
    // is its own and a second work joining it must not rename it.
    let insert_series = db.sql(
        "INSERT INTO series (id, slug, title, description, owner_pseud_id, created_at, updated_at, version)
         VALUES (?, ?, ?, '', ?, ?, ?, 1)
         ON CONFLICT(slug) DO NOTHING",
        "INSERT INTO series (id, slug, title, description, owner_pseud_id, created_at, updated_at, version)
         VALUES (?::uuid, ?, ?, '', ?::uuid, ?, ?, 1)
         ON CONFLICT(slug) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&insert_series)
                .bind(&series_id)
                .bind(slug)
                .bind(title)
                .bind(owner_pseud_id.as_deref())
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&insert_series)
                .bind(&series_id)
                .bind(slug)
                .bind(title)
                .bind(owner_pseud_id.as_deref())
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    let read_series = db.sql(
        "SELECT id FROM series WHERE slug = ?",
        "SELECT id::text FROM series WHERE slug = $1",
    );
    let existing: String = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&read_series)
                .bind(slug)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&read_series)
                .bind(slug)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    // The entry. `ON CONFLICT (series_id, work_id) DO UPDATE` so a work that
    // moves position is updated rather than duplicated — the series' order is
    // one order, and a work in it twice would be two.
    let entry_id = uuid::Uuid::new_v4().to_string();
    let insert_entry = db.sql(
        "INSERT INTO series_entries (id, series_id, work_id, position, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT(series_id, work_id) DO UPDATE SET position = excluded.position,
             updated_at = excluded.updated_at, version = series_entries.version + 1",
        "INSERT INTO series_entries (id, series_id, work_id, position, created_at, updated_at, version)
         VALUES (?::uuid, ?::uuid, ?::uuid, ?, ?, ?, 1)
         ON CONFLICT(series_id, work_id) DO UPDATE SET position = excluded.position,
             updated_at = excluded.updated_at, version = series_entries.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&insert_entry)
                .bind(&entry_id)
                .bind(&existing)
                .bind(work_id)
                .bind(position)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&insert_entry)
                .bind(&entry_id)
                .bind(&existing)
                .bind(work_id)
                .bind(position)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }

    Ok(existing)
}
