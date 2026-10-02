//! Storing and reading §50.2's canon-agnostic class (M45-31).
//!
//! ## Why this module exists and what it refuses to be
//!
//! `migrations/0106` created `canon_agnostic_works` and nothing ever wrote to it
//! or read from it: the CSV said so, in as many words, when it was honest about
//! what was missing. [`measure_work_and_store`] is the producer an ingest path
//! calls and [`stored_canon`] is the reader a discovery query calls.
//!
//! The design constraint that shapes everything here is §50.3's: *"Canon-blind
//! discovery changes eligibility, never ranking."* So this module stores a CLASS
//! and the measures behind it, and exposes no function that takes a class and
//! returns a weight. There is deliberately no `canon_affinity_score`.
//!
//! ## Absence is a third state
//!
//! §49.3 says an absent value is not a zero, and 0108 extends the table so the
//! class is stored rather than implied by the row's existence. A work therefore
//! has three states, not two:
//!
//! * classified [`CanonAgnostic`] or [`CanonDependent`], carrying the measures
//!   that produced the verdict;
//! * unclassified, carrying a reason;
//! * **no row at all**, meaning nobody has measured it.
//!
//! The last two are different and the difference matters: "measured and
//! canon-dependent" is an answer, "unclassified" is the absence of one, and "no
//! row" means the ingest path has not reached this work yet. A discovery query
//! that needs "known to be canon-dependent" must not accept either absence — see
//! [`is_canon_dependent`].

use crate::{Backend, Database, Result};
use lorehaven_domain::canon::{classify, CanonClass, CanonMeasures, MIN_CANON_WORDS};

use crate::work_coordinates::corpus_for_work;

/// A stored §50.2 verdict, with the version of the text that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCanon {
    /// The work this verdict is about.
    pub work_id: String,
    /// The class, or why there is none.
    pub verdict: CanonVerdict,
    /// Which revision of the work's text was measured.
    pub text_version: i64,
    /// When the measurement ran.
    pub declared_at: String,
}

/// §50.2's answer for one work.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonVerdict {
    /// Readable without its parent canon.
    CanonAgnostic(CanonMeasures),
    /// Carries unexplained proper nouns.
    CanonDependent(CanonMeasures),
    /// No class was reached, and here is why.
    ///
    /// §49.3's rule: this is not the same as "canon-agnostic because nothing was
    /// wrong", and a caller must not read it as one. `reason` is a short stable
    /// string, matching `work_coordinates.unmeasurable_reason`.
    Unclassified {
        /// Why no class was reached.
        reason: String,
    },
}

impl CanonVerdict {
    /// The class, or `None` when there is none.
    ///
    /// The only accessor, deliberately. A caller that needs to know whether a
    /// work is *eligible* should ask the eligibility question
    /// ([`is_canon_agnostic`]) rather than pattern-match here, because eligibility
    /// is §50.2's rule and not this enum's.
    #[must_use]
    pub fn class(&self) -> Option<CanonClass> {
        match self {
            Self::CanonAgnostic(_) => Some(CanonClass::CanonAgnostic),
            Self::CanonDependent(_) => Some(CanonClass::CanonDependent),
            Self::Unclassified { .. } => None,
        }
    }

    /// The measures behind the verdict, when there was one.
    #[must_use]
    pub fn measures(&self) -> Option<CanonMeasures> {
        match self {
            Self::CanonAgnostic(m) | Self::CanonDependent(m) => Some(*m),
            Self::Unclassified { .. } => None,
        }
    }
}

/// The reason stored for a work whose text was too short to classify.
///
/// §50.2's own threshold, named once so the reason string cannot drift from the
/// constant that produced it.
pub const TOO_SHORT: &str = "too_short";

/// The reason stored for a work with no text to classify at all.
pub const NO_TEXT: &str = "no_text";

/// Measure a work's own text, store the class, and return what was stored.
///
/// The one write path that takes a work id, so the text that was classified and
/// the text version recorded against it cannot come from different reads — the
/// same reasoning [`crate::work_coordinates::measure_work_and_store`] gives for
/// coordinates. A caller that already holds a corpus uses
/// [`store_canon_class`], which is what a backfill over many works wants.
///
/// A work with no chapters returns `Ok(None)`: nothing was classified and
/// nothing was written. Storing "unclassified" for it would be a claim that
/// somebody looked.
pub async fn measure_work_and_store(
    db: &Database,
    work_id: &str,
    computed_at: &str,
) -> Result<Option<StoredCanon>> {
    let Some((corpus, text_version)) = corpus_for_work(db, work_id, 0).await? else {
        return Ok(None);
    };
    Ok(Some(
        store_canon_class(db, work_id, &corpus, text_version, computed_at).await?,
    ))
}

/// Store an already-computed class, replacing any previous row for the work.
///
/// Upsert rather than insert-or-error, because a recompute must be safe to
/// repeat: running it twice over unchanged text yields one row with the same
/// values. That is what makes a backfill safe to re-run after a crash, and it
/// matches `work_coordinates`' upsert rather than inventing a second convention.
///
/// `word_count` comes from the corpus rather than from
/// [`CanonMeasures::word_count`], because the classifier is handed the count as
/// an input: it measures names, not words. Taking the count from the text here
/// means the stored density is always `names / (the text's own word count)`.
pub async fn store_canon_class(
    db: &Database,
    work_id: &str,
    corpus: &lorehaven_domain::coordinates::Corpus,
    text_version: i64,
    declared_at: &str,
) -> Result<StoredCanon> {
    let text = corpus.text();
    let outcome = classify(&text, corpus.word_count());
    let (verdict, measures) = match outcome {
        Some((class, measures)) => {
            let v = match class {
                CanonClass::CanonAgnostic => CanonVerdict::CanonAgnostic(measures),
                CanonClass::CanonDependent => CanonVerdict::CanonDependent(measures),
            };
            (v, Some(measures))
        }
        None => {
            // Below the threshold, so §50.2 has no answer. The two reasons are
            // distinguished because "there was no text" and "there was too
            // little" call for different follow-up: the first is an ingest bug,
            // the second is a fact about the work.
            // &'static str, so the bind below needs no clone and no conversion.
            let reason = if corpus.word_count() == 0 {
                NO_TEXT
            } else {
                TOO_SHORT
            };
            (
                CanonVerdict::Unclassified {
                    reason: reason.into(),
                },
                None,
            )
        }
    };

    let canon_dependent = match verdict {
        CanonVerdict::CanonDependent(_) => true,
        CanonVerdict::CanonAgnostic(_) | CanonVerdict::Unclassified { .. } => false,
    };
    // The reason and the measures travel together or not at all; 0108's CHECK
    // pairs them and would refuse anything else. Written as one `if` so the
    // pairing is decided once, here, rather than twice in the bind list below.
    let (reason_out, names, words, density) = match measures {
        Some(m) => (
            None,
            Some(m.unexplained_names as i64),
            Some(m.word_count as i64),
            Some(m.density),
        ),
        None => (Some(unclassified_reason(&verdict)), None, None, None),
    };

    let sql = db.sql(
        "INSERT INTO canon_agnostic_works
             (work_id, text_version, declared_at, canon_dependent, unmeasurable_reason,
              unexplained_names, word_count, density)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (work_id) DO UPDATE SET
             text_version        = excluded.text_version,
             declared_at         = excluded.declared_at,
             canon_dependent     = excluded.canon_dependent,
             unmeasurable_reason = excluded.unmeasurable_reason,
             unexplained_names   = excluded.unexplained_names,
             word_count          = excluded.word_count,
             density             = excluded.density",
        "INSERT INTO canon_agnostic_works
             (work_id, text_version, declared_at, canon_dependent, unmeasurable_reason,
              unexplained_names, word_count, density)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (work_id) DO UPDATE SET
             text_version        = excluded.text_version,
             declared_at         = excluded.declared_at,
             canon_dependent     = excluded.canon_dependent,
             unmeasurable_reason = excluded.unmeasurable_reason,
             unexplained_names   = excluded.unexplained_names,
             word_count          = excluded.word_count,
             density             = excluded.density",
    );

    match db.backend() {
        Backend::Sqlite => {
            // 0108 stores `canon_dependent` as INTEGER on SQLite, with a CHECK
            // against 0/1, so the bool must go in as 0/1 and not as `true`.
            sqlx::query(&sql)
                .bind(work_id)
                .bind(text_version)
                .bind(declared_at)
                .bind(if canon_dependent { 1_i64 } else { 0_i64 })
                .bind(reason_out)
                .bind(names)
                .bind(words)
                .bind(density)
                .execute(db.sqlite_pool().expect("sqlite pool"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work_id)
                .bind(text_version)
                .bind(declared_at)
                .bind(canon_dependent)
                .bind(reason_out)
                .bind(names)
                .bind(words)
                .bind(density)
                .execute(db.postgres_pool().expect("postgres pool"))
                .await?;
        }
    }

    Ok(StoredCanon {
        work_id: work_id.to_owned(),
        verdict,
        text_version,
        declared_at: declared_at.to_owned(),
    })
}

/// The stored reason for a verdict that has none.
///
/// Borrowed rather than cloned, so the write path binds it as `&str` and the read
/// path owns the `String` the enum holds. The `unreachable!` is not defensive
/// noise: 0108's CHECK refuses a row carrying both a class and a reason, so this
/// arm cannot be reached from a row the database accepted.
fn unclassified_reason(verdict: &CanonVerdict) -> &str {
    match verdict {
        CanonVerdict::Unclassified { reason } => reason,
        CanonVerdict::CanonAgnostic(_) | CanonVerdict::CanonDependent(_) => {
            unreachable!("a classified verdict has no reason")
        }
    }
}

type CanonRowSqlite = (
    i64,
    String,
    i64,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<f64>,
);

type CanonRowPostgres = (
    i64,
    String,
    bool,
    Option<String>,
    // `i32`, not `i64`: PostgreSQL's INTEGER is INT4 and sqlx checks the width, so
    // an `i64` arm fails on the first row with "Option<i64> (as SQL type INT8) is
    // not compatible with SQL type INT4". SQLite has no width for an INTEGER, so
    // the same column decodes as `i64` there. Another per-dialect decode, and the
    // reason these two row types are named rather than shared.
    Option<i32>,
    Option<i32>,
    Option<f64>,
);

pub async fn stored_canon(db: &Database, work_id: &str) -> Result<Option<StoredCanon>> {
    let sql = db.sql(
        "SELECT text_version, declared_at, canon_dependent, unmeasurable_reason,
                unexplained_names, word_count, density
         FROM canon_agnostic_works
         WHERE work_id = ?",
        "SELECT text_version, declared_at, canon_dependent, unmeasurable_reason,
                unexplained_names, word_count, density
         FROM canon_agnostic_works
         WHERE work_id = $1::uuid",
    );

    // A tuple rather than a FromRow struct, because `SqliteRow` and `PgRow` are
    // different types: one `let row = match backend` cannot hold either.
    let (text_version, declared_at, canon_dependent, reason, names, words, density);
    match db.backend() {
        Backend::Sqlite => {
            // `canon_dependent` is INTEGER here (0108), so sqlx decodes it as
            // `i64` and it is compared against 0. A shared decode across both
            // arms would have to pick one Rust type, and picking `i64` would
            // fail on the first row of a Postgres query, where the column is
            // BOOLEAN and sqlx decodes it as `bool`. Same shape of bug as
            // M45-24's `kudos_reason`, which read a per-dialect nullable as one
            // type and panicked on PostgreSQL.
            let row: Option<CanonRowSqlite> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite pool"))
                .await?;
            let Some((v, d, flag, r, n, w, de)) = row else {
                return Ok(None);
            };
            (
                text_version,
                declared_at,
                canon_dependent,
                reason,
                names,
                words,
                density,
            ) = (v, d, flag != 0, r, n, w, de);
        }
        Backend::Postgres => {
            // `canon_dependent` is BOOLEAN here, so sqlx decodes it as bool.
            let row: Option<CanonRowPostgres> = sqlx::query_as(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres pool"))
                .await?;
            let Some((v, d, flag, r, n, w, de)) = row else {
                return Ok(None);
            };
            // Widen the two INT4 counts to i64 so the local tuple declared above
            // holds one type on both engines, and the `as usize` below has a
            // single width to accept.
            (
                text_version,
                declared_at,
                canon_dependent,
                reason,
                names,
                words,
                density,
            ) = (v, d, flag, r, n.map(i64::from), w.map(i64::from), de);
        }
    }

    let verdict = match (reason, names, words, density) {
        (None, Some(n), Some(w), Some(d)) => {
            let measures = CanonMeasures {
                unexplained_names: n as usize,
                word_count: w as usize,
                density: d,
            };
            if canon_dependent {
                CanonVerdict::CanonDependent(measures)
            } else {
                CanonVerdict::CanonAgnostic(measures)
            }
        }
        (Some(reason), None, None, None) => CanonVerdict::Unclassified { reason },
        // 0108's CHECK makes these unreachable. `unreachable!` rather than a
        // silent default: a row like this means the constraint was dropped or a
        // migration ran out of order, and reading it as "canon-agnostic" would be
        // exactly the absent-is-not-a-zero mistake the rule forbids.
        other => {
            unreachable!("canon_agnostic_works row is neither classified nor explained: {other:?}")
        }
    };

    Ok(Some(StoredCanon {
        work_id: work_id.to_owned(),
        verdict,
        text_version,
        declared_at,
    }))
}

/// Is this work KNOWN to be canon-dependent?
///
/// The question §50.2's discovery needs, and the one place the three states are
/// resolved into a boolean. `true` only for a measured [`CanonVerdict::CanonDependent`]:
/// an unclassified work and an unmeasured one are both *not known*, which is
/// different from being known to be safe.
pub async fn is_canon_dependent(db: &Database, work_id: &str) -> Result<bool> {
    Ok(matches!(
        stored_canon(db, work_id).await?.map(|s| s.verdict),
        Some(CanonVerdict::CanonDependent(_))
    ))
}

/// Is this work KNOWN to be canon-agnostic, and therefore eligible (§50.2)?
///
/// `false` for both absences, which is the conservative direction: an unmeasured
/// work is not offered in a fandom-blind listing, because offering it is how a
/// reader meets a text that stalls them. §50.2 wants eligibility to be a
/// property of the work, not a guess.
pub async fn is_canon_agnostic(db: &Database, work_id: &str) -> Result<bool> {
    Ok(matches!(
        stored_canon(db, work_id).await?.map(|s| s.verdict),
        Some(CanonVerdict::CanonAgnostic(_))
    ))
}

/// The smallest word count §50.2 will classify, re-exported.
///
/// A discovery query filtering on `word_count >= MIN_CANON_WORDS` is asking a
/// question the schema already answers, so the constant is exposed here rather
/// than left for a caller to import from the domain crate by a path that may move.
pub const MIN_CLASSIFIABLE_WORDS: usize = MIN_CANON_WORDS;
