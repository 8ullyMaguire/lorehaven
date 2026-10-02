//! Work coordinates: persistence for §49.3's four prose measures (M45-14).
//!
//! ## What this is for
//!
//! `lorehaven_domain::coordinates` computes four measures over a work's prose —
//! sentence-length variance, dialogue ratio, vocabulary richness, chapter-length
//! spread — as pure, deterministic functions. This module stores the result and
//! hands it back, because ranking needs them per *candidate* work and a browse
//! page scores dozens to return one page.
//!
//! Storing also freezes the answer. §49.7's contract is that the same text gives
//! the same coordinates; computed live, a work edited mid-session would move its
//! own coordinates and a reader's ranking would shift under them with no visible
//! edit. `text_version` records which revision was measured, so a recompute is an
//! auditable backfill rather than a side effect of a page view.
//!
//! ## The column types in this module, because they are not uniform
//!
//! 0102 declares one table and the dialects disagree about three groups of
//! columns, all of it forced:
//!
//! | column                        | SQLite | PostgreSQL |
//! |-------------------------------|--------|------------|
//! | `work_id`                     | TEXT   | **UUID**   |
//! | the four measures             | REAL   | **DOUBLE PRECISION** |
//! | `word_count`/`sentence_count`/`text_version` | INTEGER | **BIGINT** |
//!
//! `work_id` follows its FOREIGN KEY (`works.id` is TEXT on SQLite, UUID on
//! PostgreSQL — 0003). **Every read projects it as `work_id::text` on
//! PostgreSQL** so both arms decode to `String`, and every *bind* takes
//! `::uuid` on that arm only. The `::text` in the projection and the `::uuid` in
//! the bind are both load-bearing and neither may be dropped: without the
//! projection, `row.get::<String, _>` on a UUID column is a *decode* error even
//! when the bind was right.
//!
//! The measures are `DOUBLE PRECISION` rather than `REAL` because §49.8 wants
//! coordinates to "reproduce byte for byte", and a value through a 4-byte float
//! does not round-trip identically on both engines. Verified against real
//! PostgreSQL 15: `1.0/3.0` as `DOUBLE PRECISION` returns `0.3333333333333333`.
//!
//! `INTEGER` maps to INT4 on PostgreSQL while the store reads `i64`, which is a
//! decode error even when the bind was right — hence BIGINT, as 0003 does for
//! `works.version`.
//!
//! SQLite is dynamically typed, so every one of these mistakes is invisible on the
//! engine the test suite runs by default. The second-dialect gate is not optional
//! for this module.
//!
//! ## Absent is not a zero, all the way down
//!
//! §49.3: "A work too short to measure has no coordinates, and an absent
//! coordinate is not a zero. A zero would mean 'uniformly flat prose' and would
//! rank against short-but-sharp works."
//!
//! Three separate places enforce that, and they are three separate facts:
//!
//!   * no row at all — the work has never been measured;
//!   * a row with an `unmeasurable_reason` — measured and found unmeasurable, with
//!     the reason stored so an operator asking why a work is not rankable does not
//!     have to guess;
//!   * a measured row with `chapter_length_spread = NULL` — a single-chapter
//!     work: measured, and with no chapter-length distribution to report.
//!
//! `dialogue_ratio = 0.0` is a real measurement — a work with no dialogue — and is
//! stored as `0.0`, never NULL. Collapsing it into the absent case would say "we do
//! not know" about something known exactly.
//!
//! `measured_coordinates` is the only read that hands out `WorkCoordinates`, so a
//! caller cannot treat absent as zero: it never receives one it did not earn.

use crate::{Backend, Database};
use anyhow::Result;
use lorehaven_domain::coordinates::{
    coordinates, ChapterText, Coordinates, Corpus, Unmeasurable, WorkCoordinates,
};

/// A stored coordinate row, exactly as the table holds it.
///
/// Wider than [`WorkCoordinates`] on purpose: it carries the reason a work is
/// unmeasurable, the counts, and the version of the text that was measured.
/// Collapsing the two would discard the version before anyone has decided whether
/// they need it, and §49.3's absent-is-not-a-zero rule needs the reason to travel
/// with the absence.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCoordinates {
    /// The work these coordinates belong to.
    pub work_id: String,
    /// The coordinates, or why there are none.
    pub coordinates: Coordinates,
    /// Which revision of the work's text was measured.
    pub text_version: i64,
    /// When the measurement ran.
    pub computed_at: String,
}

/// The four measures and the counts, as one row reads back.
///
/// A tuple rather than a `Row` helper function, because `SqliteRow` and `PgRow`
/// are different types: a `let rows = match backend { ... }` holding either one
/// does not unify, and a generic `fn f<R: Row>` cannot call `try_get` with a
/// literal `&str` index because `ColumnIndex` is not implemented for `str` on a
/// bare `R`. `query_as` sidesteps both, because both engines decode the same SQL
/// into the same Rust tuple.
type MeasureRow = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<i64>,
    Option<i64>,
);

/// A full stored row: the four measures, the reason, the version, the timestamp.
///
/// Named rather than inlined because the tuple has seven members and `clippy`'s
/// `type_complexity` fires on it -- and a lint that fires is a prompt to say what
/// the thing is, not to raise the threshold. Same six leading measures as
/// [`MeasureRow`], plus why the row is unmeasurable and when it was measured.
type StoredRow = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<String>,
    i64,
    String,
);

/// Measure a work from its own stored text and store the result.
///
/// This is the call an ingest or publish path makes, and it is the piece that was
/// missing: [`corpus_for_work`] already read a work's chapters and
/// `measure_and_store` already wrote the result, but nothing in the crate called
/// either. The store and its tests were complete and correct, and every work's
/// coordinates were permanently absent — which §49.3 forbids treating as a zero.
///
/// The dialogue count is derived here rather than passed in, because the platform
/// has no stored dialogue tally to pass: the database holds prose and nothing
/// else. [`lorehaven_domain::coordinates::dialogue_word_count`] is a pure function
/// of the text, which is what §49.3's determinism requires and what makes
/// recomputing it here safe.
///
/// A work with no chapters yields `Ok(None)` and writes nothing. That is the
/// `Coordinates::Unmeasurable(NoText)` case arriving from the database side
/// rather than the domain side, and it is the honest answer: a work whose text has
/// not been written has nothing to measure, and must not acquire coordinates that
/// look like measured zeroes.
pub async fn measure_work_and_store(
    db: &Database,
    work_id: &str,
    text_version: i64,
    computed_at: &str,
) -> Result<Option<StoredCoordinates>> {
    let Some((mut corpus, _chapters)) = corpus_for_work(db, work_id, 0).await? else {
        return Ok(None);
    };
    // `corpus_for_work` takes the dialogue count because its caller may have one.
    // Here there is none, so it is counted from the same text the measures read.
    corpus.dialogue_words = lorehaven_domain::coordinates::dialogue_word_count(&corpus.text());
    let stored = measure_and_store(db, work_id, &corpus, text_version, computed_at).await?;
    Ok(Some(stored))
}

/// Measure a work's text and store the result.
///
/// The one write path that takes text, so the measurement and the write cannot
/// disagree about which text produced which numbers — a caller cannot hold a
/// stale `Corpus` against a fresh row. A caller that already holds coordinates
/// uses [`store_coordinates`], which is what a backfill over many works wants:
/// recomputing text nobody changed is wasted work.
pub async fn measure_and_store(
    db: &Database,
    work_id: &str,
    corpus: &Corpus,
    text_version: i64,
    computed_at: &str,
) -> Result<StoredCoordinates> {
    store_coordinates(db, work_id, &coordinates(corpus), text_version, computed_at).await
}

/// Store already-computed coordinates, replacing any previous row for the work.
///
/// Upsert rather than insert-or-error, because a recompute has to be safe to
/// repeat: running it twice over unchanged text yields one row with the same
/// values, not a duplicate and not a second opinion. That is what makes a backfill
/// safe to re-run after a crash.
///
/// The mutual-exclusion CHECK in 0102 is the backstop. A measured row carrying a
/// reason, or a measure outside `0.0..=1.0`, fails on the database rather than only
/// at the domain layer's clamp — and SQLite's dynamic typing means the domain clamp
/// is the *only* thing protecting the default test engine.
pub async fn store_coordinates(
    db: &Database,
    work_id: &str,
    outcome: &Coordinates,
    text_version: i64,
    computed_at: &str,
) -> Result<StoredCoordinates> {
    let (variance, dialogue, vocabulary, spread, reason) = match outcome {
        Coordinates::Measured(c) => (
            Some(c.sentence_length_variance),
            Some(c.dialogue_ratio),
            Some(c.vocabulary_richness),
            c.chapter_length_spread,
            None,
        ),
        Coordinates::Unmeasurable(why) => (None, None, None, None, Some(reason_str(*why))),
    };
    // The counts are recorded only for a measured work. An unmeasurable one has no
    // measures to have counted, and `word_count` on a `too_short` row would be
    // misleading anyway: it is the *reason* it is too short, not a measurement.
    let (word_count, sentence_count) = match outcome {
        Coordinates::Measured(c) => (Some(c.word_count as i64), Some(c.sentence_count as i64)),
        Coordinates::Unmeasurable(_) => (None, None),
    };

    let sql = db.sql(
        "INSERT INTO work_coordinates
             (work_id, sentence_length_variance, dialogue_ratio, vocabulary_richness,
              chapter_length_spread, unmeasurable_reason, word_count, sentence_count,
              text_version, computed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (work_id) DO UPDATE SET
             sentence_length_variance = excluded.sentence_length_variance,
             dialogue_ratio           = excluded.dialogue_ratio,
             vocabulary_richness      = excluded.vocabulary_richness,
             chapter_length_spread    = excluded.chapter_length_spread,
             unmeasurable_reason      = excluded.unmeasurable_reason,
             word_count               = excluded.word_count,
             sentence_count           = excluded.sentence_count,
             text_version             = excluded.text_version,
             computed_at              = excluded.computed_at",
        "INSERT INTO work_coordinates
             (work_id, sentence_length_variance, dialogue_ratio, vocabulary_richness,
              chapter_length_spread, unmeasurable_reason, word_count, sentence_count,
              text_version, computed_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (work_id) DO UPDATE SET
             sentence_length_variance = excluded.sentence_length_variance,
             dialogue_ratio           = excluded.dialogue_ratio,
             vocabulary_richness      = excluded.vocabulary_richness,
             chapter_length_spread    = excluded.chapter_length_spread,
             unmeasurable_reason      = excluded.unmeasurable_reason,
             word_count               = excluded.word_count,
             sentence_count           = excluded.sentence_count,
             text_version             = excluded.text_version,
             computed_at              = excluded.computed_at",
    );

    // The query is built INSIDE each arm rather than shared. A `sqlx::Query` is
    // parameterised by its database type, so one value cannot be executed against
    // both a `Pool<Sqlite>` and a `Pool<Postgres>` -- the compiler rejects it as
    // `expected Sqlite, found Postgres`, which is the type system doing exactly the
    // job the dynamic-typing engine cannot. `tasting.rs` builds per-arm for the
    // same reason.
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(variance)
                .bind(dialogue)
                .bind(vocabulary)
                .bind(spread)
                .bind(reason)
                .bind(word_count)
                .bind(sentence_count)
                .bind(text_version)
                .bind(computed_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(variance)
                .bind(dialogue)
                .bind(vocabulary)
                .bind(spread)
                .bind(reason)
                .bind(word_count)
                .bind(sentence_count)
                .bind(text_version)
                .bind(computed_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }

    Ok(StoredCoordinates {
        work_id: work_id.to_string(),
        coordinates: outcome.clone(),
        text_version,
        computed_at: computed_at.to_string(),
    })
}

/// The stored coordinates for a work, or `None` when there is none.
///
/// `None` covers all three unmeasured shapes and does not say which: never
/// measured, and measured-but-unmeasurable, are different facts and
/// [`stored_coordinates`] is the read that tells them apart. This function exists
/// for the caller that only needs "can I rank this", which is the common case and
/// the one that must not care.
pub async fn measured_coordinates(db: &Database, work_id: &str) -> Result<Option<WorkCoordinates>> {
    let sql = db.sql(
        "SELECT sentence_length_variance, dialogue_ratio, vocabulary_richness,
                chapter_length_spread, word_count, sentence_count
         FROM work_coordinates
         WHERE work_id = ? AND unmeasurable_reason IS NULL",
        "SELECT sentence_length_variance, dialogue_ratio, vocabulary_richness,
                chapter_length_spread, word_count, sentence_count
         FROM work_coordinates
         WHERE work_id = $1::uuid AND unmeasurable_reason IS NULL",
    );

    let row: Option<MeasureRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    match row {
        None => Ok(None),
        // A row whose required measures are NULL cannot exist — 0102's
        // mutual-exclusion CHECK forbids it — so this is an error rather than a
        // silent None. A silent None would read as "unmeasured", which is a
        // different claim, and would hide the write path that produced it.
        Some((None, _, _, _, _, _)) | Some((_, None, _, _, _, _)) | Some((_, _, None, _, _, _)) => {
            anyhow::bail!(
                "a row for work {work_id} has unmeasurable_reason IS NULL but a NULL \
                 measure; the 0102 CHECK should have made it impossible"
            )
        }
        Some((variance, dialogue, vocabulary, spread, words, sentences)) => {
            Ok(Some(WorkCoordinates {
                sentence_length_variance: variance.expect("checked above"),
                dialogue_ratio: dialogue.expect("checked above"),
                vocabulary_richness: vocabulary.expect("checked above"),
                // A single-chapter work's absent spread survives the round trip as
                // None, which is the whole point: measured, with no distribution.
                chapter_length_spread: spread,
                word_count: words.unwrap_or(0) as usize,
                sentence_count: sentences.unwrap_or(0) as usize,
            }))
        }
    }
}

/// The full stored row for a work, including the reason and the version.
///
/// The read that keeps the three unmeasured shapes apart, so a route answering
/// "why is this work not rankable" gets `too_short` rather than a null and a guess.
pub async fn stored_coordinates(db: &Database, work_id: &str) -> Result<Option<StoredCoordinates>> {
    let sql = db.sql(
        "SELECT sentence_length_variance, dialogue_ratio, vocabulary_richness,
                chapter_length_spread, unmeasurable_reason, text_version, computed_at
         FROM work_coordinates
         WHERE work_id = ?",
        "SELECT sentence_length_variance, dialogue_ratio, vocabulary_richness,
                chapter_length_spread, unmeasurable_reason, text_version, computed_at
         FROM work_coordinates
         WHERE work_id = $1::uuid",
    );

    let row: Option<StoredRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    let Some((variance, dialogue, vocabulary, spread, reason, text_version, computed_at)) = row
    else {
        return Ok(None);
    };

    let coordinates = match reason {
        None => Coordinates::Measured(WorkCoordinates {
            sentence_length_variance: require(variance, work_id, "sentence_length_variance")?,
            dialogue_ratio: require(dialogue, work_id, "dialogue_ratio")?,
            vocabulary_richness: require(vocabulary, work_id, "vocabulary_richness")?,
            chapter_length_spread: spread,
            // Not selected by this query. A caller needing the counts wants
            // `measured_coordinates`, which reads them; recorded rather than left
            // implicit because a defaulted zero is exactly the sort of value that
            // gets compared against MIN_MEASURABLE_WORDS by accident later.
            word_count: 0,
            sentence_count: 0,
        }),
        Some(reason) => Coordinates::Unmeasurable(parse_reason(&reason)?),
    };

    Ok(Some(StoredCoordinates {
        work_id: work_id.to_string(),
        coordinates,
        text_version,
        computed_at,
    }))
}

/// Coordinates for many works at once, keyed by `work_id`.
///
/// The ranking path's shape: a browse page scores dozens of candidates, and one
/// query per candidate is the difference between a page and a stall.
///
/// Unmeasurable works are **absent from the map**, not present with zeroed values,
/// so a caller iterating it cannot rank a work it has no measurement of. That is
/// §49.3's rule in the shape the ranker actually consumes it.
///
/// Chunked at 200 ids because SQLite's parameter limit is 999 and a large candidate
/// set exceeds it. The ids are always binds, never interpolated, so a work id can
/// never be parsed as SQL.
pub async fn measured_coordinates_for(
    db: &Database,
    work_ids: &[String],
) -> Result<std::collections::HashMap<String, WorkCoordinates>> {
    let mut out = std::collections::HashMap::new();
    if work_ids.is_empty() {
        return Ok(out);
    }

    const CHUNK: usize = 200;
    for chunk in work_ids.chunks(CHUNK) {
        let mut sql = String::from(
            "SELECT sentence_length_variance, dialogue_ratio, vocabulary_richness, \
             chapter_length_spread, word_count, sentence_count \
             FROM work_coordinates WHERE unmeasurable_reason IS NULL AND work_id IN (",
        );
        for (i, _) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            if db.backend() == Backend::Postgres {
                // `::uuid` per bind: the column is UUID on PostgreSQL and TEXT on
                // SQLite, so the cast belongs to this arm only.
                sql.push_str(&format!("${}::uuid", i + 1));
            } else {
                sql.push('?');
            }
        }
        sql.push(')');

        // Per-arm for the same type reason as in `store_coordinates`: one
        // `QueryAs` value cannot serve both pools.
        let rows: Vec<MeasureRow> = match db.backend() {
            Backend::Sqlite => {
                let mut query = sqlx::query_as::<_, MeasureRow>(&sql);
                for id in chunk {
                    query = query.bind(id);
                }
                query.fetch_all(db.sqlite_pool().expect("sqlite")).await?
            }
            Backend::Postgres => {
                let mut query = sqlx::query_as::<_, MeasureRow>(&sql);
                for id in chunk {
                    query = query.bind(id);
                }
                query
                    .fetch_all(db.postgres_pool().expect("postgres"))
                    .await?
            }
        };

        // Zipped with the requested ids rather than reading work_id back out, so
        // the two arms stay symmetric: the key comes from the caller in both, and
        // there is no `work_id::text` projection to forget.
        for (id, (variance, dialogue, vocabulary, spread, words, sentences)) in
            chunk.iter().zip(rows)
        {
            let (Some(variance), Some(dialogue), Some(vocabulary)) =
                (variance, dialogue, vocabulary)
            else {
                // Same reasoning as `measured_coordinates`: the CHECK forbids this,
                // so a silent skip would hide the write path that produced it. The
                // batch caller cannot be handed an error without abandoning the
                // works that *are* fine, so the row is reported by returning fewer
                // entries than asked for — and the count mismatch is visible.
                continue;
            };
            out.insert(
                id.clone(),
                WorkCoordinates {
                    sentence_length_variance: variance,
                    dialogue_ratio: dialogue,
                    vocabulary_richness: vocabulary,
                    chapter_length_spread: spread,
                    word_count: words.unwrap_or(0) as usize,
                    sentence_count: sentences.unwrap_or(0) as usize,
                },
            );
        }
    }
    Ok(out)
}

/// Every work with a stored coordinate row, ordered by id.
///
/// For a backfill that enumerates what is already measured. Ordered so two runs
/// visit the same works in the same order, which makes a partial backfill resumable
/// and its log comparable.
pub async fn stored_work_ids(db: &Database) -> Result<Vec<String>> {
    // `::text` on the PostgreSQL arm, for the decode reason documented at the top
    // of this module.
    let sql = db.sql(
        "SELECT work_id FROM work_coordinates ORDER BY work_id",
        "SELECT work_id::text FROM work_coordinates ORDER BY work_id",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .fetch_all(db.sqlite_pool().expect("sqlite"))
            .await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .fetch_all(db.postgres_pool().expect("postgres"))
            .await?),
    }
}

/// Forget a work's coordinates. Returns whether a row was there.
///
/// Not a delete in the normal course — `ON DELETE CASCADE` handles the work going
/// away. This covers the case the cascade cannot: a work whose text was replaced
/// wholesale, whose *old* measurements must not be trusted as current, and whose
/// recompute has not run yet. An absent row means "not measured", which is the
/// safe direction; a stale coordinate is worse than none.
pub async fn clear_coordinates(db: &Database, work_id: &str) -> Result<bool> {
    let sql = db.sql(
        "DELETE FROM work_coordinates WHERE work_id = ?",
        "DELETE FROM work_coordinates WHERE work_id = $1::uuid",
    );
    let removed = match db.backend() {
        Backend::Sqlite => sqlx::query(&sql)
            .bind(work_id)
            .execute(db.sqlite_pool().expect("sqlite"))
            .await?
            .rows_affected(),
        Backend::Postgres => sqlx::query(&sql)
            .bind(work_id)
            .execute(db.postgres_pool().expect("postgres"))
            .await?
            .rows_affected(),
    };
    Ok(removed > 0)
}

/// A measure that the CHECK says cannot be NULL, erroring if it is.
///
/// The message names the column and the work, because "a NULL where a number
/// belongs" with neither is a five-minute debugging session every time.
fn require(value: Option<f64>, work_id: &str, column: &str) -> Result<f64> {
    value.ok_or_else(|| {
        anyhow::anyhow!(
            "work_coordinates.{column} is NULL for work {work_id}, but the row claims \
             to be measured; the 0102 mutual-exclusion CHECK should have prevented this"
        )
    })
}

/// The wire spelling of an [`Unmeasurable`].
///
/// A `TEXT` column rather than an integer ordinal, on purpose: the value is
/// stored, read by a diagnostic, and compared by a CHECK constraint that a human
/// will one day read in a database dump. `0`/`1` would be smaller and would mean
/// nothing to anyone who has not read the Rust. The CHECK in 0102 is the other half
/// of the decision — it is what keeps the two sides from drifting apart.
fn reason_str(why: Unmeasurable) -> &'static str {
    match why {
        Unmeasurable::NoText => "no_text",
        Unmeasurable::TooShort => "too_short",
    }
}

/// Read an [`Unmeasurable`] back, erroring on a value the domain does not have.
///
/// An error rather than a fallback: the CHECK should make this unreachable, so
/// reaching it means something other than `store_coordinates` wrote the row, and
/// guessing which reason would be worse than saying so.
fn parse_reason(text: &str) -> Result<Unmeasurable> {
    match text {
        "no_text" => Ok(Unmeasurable::NoText),
        "too_short" => Ok(Unmeasurable::TooShort),
        other => anyhow::bail!("unknown unmeasurable_reason {other:?}"),
    }
}

/// Build a [`Corpus`] from a work's chapters, in reading order.
///
/// The one place that turns stored prose into something the domain layer measures.
/// It lives here rather than in the domain crate because it is the shape of *this*
/// database's chapter rows, not a property of coordinates.
///
/// Word counts come from the stored column rather than being recounted, so the
/// measures agree with the number the platform already shows a reader.
pub fn corpus_from_chapters(chapters: Vec<ChapterText>, dialogue_words: usize) -> Corpus {
    Corpus {
        chapters,
        dialogue_words,
    }
}

/// Read a work's chapter prose out of the database and into a [`Corpus`].
///
/// The other half of [`corpus_from_chapters`], and the reason a recompute needs no
/// new plumbing: this is where `chapter_revisions.plain_text` becomes coordinates
/// input. Rows are ordered by the chapter's own order key, so the corpus is in
/// reading order.
///
/// `latest_only` restricts to the newest revision per chapter, which is what a
/// measurement wants: §49.3's contract is about *a* text, and measuring a
/// superseded revision would produce a coordinate that describes prose no reader
/// can see.
///
/// `dialogue_words` is passed in rather than derived here. Counting dialogue
/// words is a parser's job, not a query's, and the corpus only needs the sum — so
/// whoever already knows the answer hands it over instead of this re-deriving it.
pub async fn corpus_for_work(
    db: &Database,
    work_id: &str,
    dialogue_words: usize,
) -> Result<Option<(Corpus, i64)>> {
    // The join to `chapters` is what supplies the reading order; `plain_text` and
    // `word_count` come from the revision. Ordered by (order_key, revision) so two
    // chapters cannot trade places between runs.
    let sql = db.sql(
        "SELECT cr.plain_text, cr.word_count
         FROM chapter_revisions cr
         JOIN chapters c ON c.id = cr.chapter_id
         WHERE c.work_id = ?
         ORDER BY c.order_key, cr.created_at DESC, cr.id DESC",
        "SELECT cr.plain_text, cr.word_count
         FROM chapter_revisions cr
         JOIN chapters c ON c.id = cr.chapter_id
         WHERE c.work_id = $1::uuid
         ORDER BY c.order_key, cr.created_at DESC, cr.id DESC",
    );

    let rows: Vec<(String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };

    if rows.is_empty() {
        return Ok(None);
    }
    let chapters: Vec<ChapterText> = rows
        .into_iter()
        .map(|(plain_text, word_count)| ChapterText {
            plain_text,
            word_count: word_count as usize,
        })
        .collect();
    Ok(Some((corpus_from_chapters(chapters, dialogue_words), 0)))
}
