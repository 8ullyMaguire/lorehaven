//! M14 — Governance repository: trust, reports, quorum, appeals, sanctions.

use serde_json::Value;
use sqlx::Row;

use darkhold_domain::governance::TL_NEW;

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

// ---------------------------------------------------------------------------
// The earned side (M3-02)
// ---------------------------------------------------------------------------

/// Read the four axes of one account's evidence (spec §14.4).
///
/// **Every axis is reviewed conduct, and the code says which review each one
/// came through.** That is the whole of the specification's rule — "trust is
/// earned through reviewed conduct, not post count, not word count, not
/// kudos, not credits, not any activity-volume metric" — and it is enforced
/// by *what is counted*, not by a comment:
///
/// | axis | what it counts | why it is reviewed conduct |
/// |---|---|---|
/// | tenure days | days since the account was created | time, which nobody can rush |
/// | accepted contributions | proposals **passed** or **approved** | somebody else accepted them |
/// | vote score | the public ratings on the account's published work | readers rated it |
/// | report accuracy | reports **upheld** minus reports **dismissed** | a reviewer decided |
///
/// The activity volume is counted too — **and then thrown away**. It is here
/// so the counter-example can be measured rather than asserted: a reader with
/// ten thousand posts and no reviewed contributions must land on a lower level
/// than a reader with twenty accepted contributions.
pub async fn evidence_for(
    db: &Database,
    account: &str,
) -> Result<darkhold_domain::trust::TrustEvidence, sqlx::Error> {
    let tenure_days = tenure_days(db, account).await?;
    let accepted_contributions = accepted_contributions(db, account).await?;
    let vote_score = vote_score(db, account).await?;
    let report_accuracy = report_accuracy(db, account).await?;
    let activity_volume = activity_volume(db, account).await?;

    Ok(darkhold_domain::trust::TrustEvidence {
        tenure_days,
        accepted_contributions,
        vote_score,
        report_accuracy,
        activity_volume,
    })
}

/// Days since the account was created.
async fn tenure_days(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT created_at FROM accounts WHERE id = ?",
        "SELECT created_at FROM accounts WHERE id = ?::uuid",
    );
    let created: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let Some((created,)) = created else {
        return Ok(0);
    };
    Ok(days_since(&created))
}

/// Whole days between an RFC 3339 timestamp and now.
///
/// **Whole days, and never negative.** An account created a moment ago has
/// zero days of tenure, not a negative one — and a clock that has moved
/// backwards (which happens, on a machine whose clock is corrected) must not
/// make a reader's tenure negative and their trust fall.
fn days_since(timestamp: &str) -> i64 {
    let Ok(at) =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
    else {
        return 0;
    };
    let now = time::OffsetDateTime::now_utc();
    let seconds = (now - at).whole_seconds();
    if seconds <= 0 {
        return 0;
    }
    seconds / 86_400
}

/// The proposals this account made that **somebody else accepted**.
///
/// **Four channels, and each one names a review.** A proposal that is still
/// open counts for nothing, because nothing has been accepted yet — which is
/// what makes this axis *reviewed conduct* and not activity.
async fn accepted_contributions(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    // `category_proposals` and `entry_moderation_proposals` record their
    // author as an account id in a TEXT column on both engines;
    // `retention_proposals` as an account id in a native id column;
    // `tag_wrangling_proposals` as a **pseudonym**, so it joins through
    // `pseuds` to reach the account.
    let queries: [(&str, &str); 4] = [
        (
            "SELECT COUNT(*) FROM category_proposals
              WHERE created_by = ? AND status = 'passed'",
            "SELECT COUNT(*) FROM category_proposals
              WHERE created_by = $1 AND status = 'passed'",
        ),
        (
            "SELECT COUNT(*) FROM entry_moderation_proposals
              WHERE created_by = ? AND status = 'passed'",
            "SELECT COUNT(*) FROM entry_moderation_proposals
              WHERE created_by = $1 AND status = 'passed'",
        ),
        (
            "SELECT COUNT(*) FROM retention_proposals
              WHERE opened_by = ? AND state = 'passed'",
            "SELECT COUNT(*) FROM retention_proposals
              WHERE opened_by = $1::uuid AND state = 'passed'",
        ),
        (
            "SELECT COUNT(*) FROM tag_wrangling_proposals
              WHERE proposed_by IN (SELECT id FROM pseuds WHERE account_id = ?)
                AND status = 'approved'",
            "SELECT COUNT(*) FROM tag_wrangling_proposals
              WHERE proposed_by IN (SELECT id FROM pseuds WHERE account_id = $1::uuid)
                AND status = 'approved'",
        ),
    ];

    let mut total = 0i64;
    for (sqlite_sql, postgres_sql) in queries {
        let sql = db.sql(sqlite_sql, postgres_sql);
        let count: (i64,) = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_as(&sql)
                    .bind(account)
                    .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_as(&sql)
                    .bind(account)
                    .fetch_one(db.postgres_pool().expect("postgres handle"))
                    .await?
            }
        };
        total += count.0;
    }
    Ok(total)
}

/// The vote score the account's published work received.
///
/// **The public ratings only.** A reader's private one-star is their own
/// business, and letting it move a writer's trust would make the axis a
/// weapon.
///
/// The score is the **excess over neutral**: a three-star rating contributes
/// nothing, a five-star contributes two and a one-star contributes minus two.
/// So a work nobody rated and a work everybody rated three stars both score
/// zero, and the axis measures *appreciation* rather than *popularity*.
async fn vote_score(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT COALESCE(SUM(r.stars), 0) - 3 * COUNT(r.id)
           FROM rating r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE p.account_id = ? AND w.lifecycle = 'published'
            AND r.is_public = 1 AND r.deleted_at IS NULL",
        "SELECT COALESCE(SUM(r.stars), 0) - 3 * COUNT(r.id)
           FROM rating r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds p ON p.id = w.owner_pseud_id
          WHERE p.account_id = $1::uuid AND w.lifecycle = 'published'
            AND r.is_public = 1 AND r.deleted_at IS NULL",
    );
    let score: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(score.0)
}

/// The account's reports that were upheld, minus the ones that were dismissed.
///
/// **Net, not a rate.** A rate would reward a reader with one lucky report as
/// much as a reader with fifty confirmed ones, and a rate over no reports is
/// a division by zero that quietly becomes "perfect".
async fn report_accuracy(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT
            COUNT(CASE WHEN LOWER(t.outcome) = 'uphold' THEN 1 END)
          - COUNT(CASE WHEN LOWER(t.outcome) = 'dismiss' THEN 1 END)
           FROM reports r
           JOIN review_tasks t ON t.report_id = r.id
          WHERE r.reporter = ?",
        "SELECT
            COUNT(CASE WHEN LOWER(t.outcome) = 'uphold' THEN 1 END)
          - COUNT(CASE WHEN LOWER(t.outcome) = 'dismiss' THEN 1 END)
           FROM reports r
           JOIN review_tasks t ON t.report_id = r.id
          WHERE r.reporter = $1::uuid",
    );
    let accuracy: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(accuracy.0)
}

/// How much the account has done — **counted, and then thrown away**.
///
/// This is the counter-example's own number. It is measured so that a test
/// can say "ten thousand posts is not a level", rather than merely asserting
/// it. The score function never receives it.
async fn activity_volume(db: &Database, account: &str) -> Result<i64, sqlx::Error> {
    let sql = db.sql(
        "SELECT
            (SELECT COUNT(*) FROM comments c
              JOIN pseuds p ON p.id = c.author_pseud
             WHERE p.account_id = ? AND c.deleted_at IS NULL)
          + (SELECT COUNT(*) FROM works w
              JOIN pseuds p ON p.id = w.owner_pseud_id
             WHERE p.account_id = ? AND w.deleted_at IS NULL)",
        "SELECT
            (SELECT COUNT(*) FROM comments c
              JOIN pseudos p ON p.id = c.author_pseud
             WHERE p.account_id = $1::uuid AND c.deleted_at IS NULL)
          + (SELECT COUNT(*) FROM works w
              JOIN pseudos p ON p.id = w.owner_pseud_id
             WHERE p.account_id = $1::uuid AND w.deleted_at IS NULL)",
    );
    let volume: (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .bind(account)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .bind(account)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(volume.0)
}

/// Recompute an account's trust level and record the event.
///
/// **The level is written with its basis**, so a level can be explained: the
/// basis carries the four axis values and the weights that produced the score
/// (spec §14.4). A level with no basis is a level somebody has to take on
/// trust.
///
/// **And the event is written in the same breath.** A level that changed with
/// no event is a defect (spec §6.7), and the cheapest way to guarantee there
/// is no such defect is to make the two writes inseparable: there is one
/// function that changes a level, and it writes both.
/// Everything the trust computation needs from the instance's configuration.
///
/// **A struct rather than five parameters**, because the fifth one would be
/// the sixth and the call site would be unreadable — and because the settings
/// travel together: the ladder without the axes is a ladder nobody can score
/// against.
#[derive(Debug, Clone, Copy)]
pub struct TrustSettings<'a> {
    /// The ladder: the levels and what each unlocks.
    pub ladder: &'a darkhold_domain::trust::TrustLadder,
    /// The axes and their weights.
    pub axes: &'a darkhold_domain::trust::TrustAxes,
    /// The operator's per-privilege requirement.
    pub requirements: &'a darkhold_domain::trust::TrustRequirements,
    /// The elastic policy.
    pub policy: darkhold_domain::trust::ElasticPolicy,
    /// The level a guest holds.
    pub guest_level: i64,
}

/// Recompute an account's trust level, record the event, and re-evaluate the
/// privilege requirements if the pool moved.
pub async fn recompute_trust(
    db: &Database,
    account: &str,
    settings: &TrustSettings<'_>,
) -> Result<darkhold_domain::trust::TrustAssessment, sqlx::Error> {
    let ladder = settings.ladder;
    let axes = settings.axes;
    let guest_level = settings.guest_level;
    let evidence = evidence_for(db, account).await?;
    let mut assessment = darkhold_domain::trust::assess(&evidence, axes, ladder);

    // **A guest never extends its trust** (spec §14.1), and this is where that
    // is enforced rather than merely configured. A guest with ninety days of
    // tenure would otherwise reach level 2 on the tenure axis alone, and the
    // guest door would be a trust farm — which is exactly what the
    // specification forbids.
    if is_guest(db, account).await? {
        assessment.level = guest_level;
        assessment.name = ladder.name_of(guest_level).unwrap_or("new").to_owned();
    }

    // The *stored* level, which is `None` for an account that has never had
    // one computed. The difference matters: an account's **first** level is an
    // initial state, not a change, and the invariant is about *changes*.
    let previous = stored_level(db, account).await?;
    // The stored basis carries the axis values the previous level was computed
    // from, so the cause of a move can be **named** rather than guessed.
    let previous_evidence = stored_evidence(db, account).await?;

    write_trust(
        db,
        account,
        assessment.level,
        &assessment.basis().to_string(),
    )
    .await?;

    let mut moved = false;
    if let Some(previous) = previous {
        if previous != assessment.level {
            crate::trust_events::record(
                db,
                account,
                previous,
                assessment.level,
                cause_for(previous_evidence.as_ref(), &evidence, axes),
                None,
                None,
            )
            .await?;
            moved = true;
        }
    }

    // **The pool moved, so the requirements are re-evaluated.** This is "the
    // next evaluation" (spec §14.4): the requirement is a stored thing, and
    // what it depends on — who holds what — has just changed.
    if moved {
        evaluate_requirements(db, ladder, settings.requirements, &settings.policy).await?;
    }
    // A level that did not change is **not** an event: an event for it would
    // make the log say that something happened when nothing did. The basis is
    // still refreshed, because the axis values behind the level did move.

    Ok(assessment)
}

/// Whether the account is a guest (spec §14.1, D-6).
async fn is_guest(db: &Database, account: &str) -> Result<bool, sqlx::Error> {
    let sql = db.sql(
        "SELECT is_guest FROM accounts WHERE id = ?",
        "SELECT is_guest::int::bigint FROM accounts WHERE id = ?::uuid",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(flag,)| flag != 0).unwrap_or(false))
}

/// The account's stored level, if it has one.
async fn stored_level(db: &Database, account: &str) -> Result<Option<i64>, sqlx::Error> {
    let sql = db.sql(
        "SELECT level FROM trust_levels WHERE account = ?",
        "SELECT CAST(level AS BIGINT) FROM trust_levels WHERE account = $1::uuid",
    );
    let row: Option<(i64,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(level,)| level))
}

/// Write the level, its basis, and the cache the tier gate reads.
///
/// **Both writes, here, in one place.** `trust_levels` is the authority
/// because it carries `computed_at` and `basis` — the level and the reason it
/// is what it is. `accounts.trust_level` is the cache the tier gate reads on
/// every request. Two tables holding one number is only safe when there is
/// exactly one writer, and this is it.
pub async fn write_trust(
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
            sqlx::query("UPDATE accounts SET trust_level = ? WHERE id = ?")
                .bind(level)
                .bind(account)
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
            sqlx::query("UPDATE accounts SET trust_level = $1 WHERE id = $2::uuid")
                .bind(level as i32)
                .bind(account)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Read the axis values the account's stored level was computed from.
///
/// **Out of the basis, not out of the columns.** The basis is the only place
/// the previous axis values exist — the level table keeps one row per account
/// and no history — so the basis is what makes "which axis moved" answerable
/// after the fact.
async fn stored_evidence(
    db: &Database,
    account: &str,
) -> Result<Option<darkhold_domain::trust::TrustEvidence>, sqlx::Error> {
    let sql = db.sql(
        "SELECT basis FROM trust_levels WHERE account = ?",
        "SELECT basis::text FROM trust_levels WHERE account = $1::uuid",
    );
    let basis: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let Some((basis,)) = basis else {
        return Ok(None);
    };
    let parsed: serde_json::Value = serde_json::from_str(&basis).unwrap_or(serde_json::Value::Null);
    let Some(evidence) = parsed.get("evidence") else {
        return Ok(None);
    };
    Ok(Some(darkhold_domain::trust::TrustEvidence {
        tenure_days: evidence["tenure_days"].as_i64().unwrap_or(0),
        accepted_contributions: evidence["accepted_contributions"].as_i64().unwrap_or(0),
        vote_score: evidence["vote_score"].as_i64().unwrap_or(0),
        report_accuracy: evidence["report_accuracy"].as_i64().unwrap_or(0),
        // The activity volume is deliberately **not** read back: it is not an
        // axis, and reading it here would put it in the basis.
        activity_volume: 0,
    }))
}

/// The cause of a level change, from the axis that moved it most.
///
/// **One event, naming one cause.** An event that said "several things
/// changed" would be an event nobody could act on — and the axes move
/// together all the time, because a reader who accepts a contribution is
/// usually also a reader whose tenure passed a week.
///
/// **The axis that moved the most, by weighted size**, and a *fall* names the
/// axis that fell. The automatic spam demotion is **not** here: that is a
/// separate rule with its own cause (spec §14.4), and it moves a level
/// deliberately rather than as a side effect of a recomputation.
fn cause_for(
    previous: Option<&darkhold_domain::trust::TrustEvidence>,
    next: &darkhold_domain::trust::TrustEvidence,
    axes: &darkhold_domain::trust::TrustAxes,
) -> crate::trust_events::TrustCause {
    let Some(previous) = previous else {
        // The first level: there is nothing to compare against, so the axis
        // that is furthest along is the one that earned it.
        return crate::trust_events::TrustCause::Tenure;
    };

    let deltas = [
        (
            (next.tenure_days - previous.tenure_days) as f64 * axes.tenure_days,
            crate::trust_events::TrustCause::Tenure,
        ),
        (
            (next.accepted_contributions - previous.accepted_contributions) as f64
                * axes.accepted_contributions,
            crate::trust_events::TrustCause::Contribution,
        ),
        (
            (next.vote_score - previous.vote_score) as f64 * axes.vote_score,
            crate::trust_events::TrustCause::VoteScore,
        ),
        (
            (next.report_accuracy - previous.report_accuracy) as f64 * axes.report_accuracy,
            crate::trust_events::TrustCause::ReportAccuracy,
        ),
    ];

    deltas
        .iter()
        .fold(
            (0.0_f64, crate::trust_events::TrustCause::Tenure),
            |best, (value, cause)| {
                if value.abs() > best.0.abs() {
                    (*value, *cause)
                } else {
                    best
                }
            },
        )
        .1
}

// ---------------------------------------------------------------------------
// The elastic allocation (M3-04)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The one automatic demotion (M3-06)
// ---------------------------------------------------------------------------

/// The confirmed spam reports against an account in the window.
///
/// **Confirmed** means a reviewer upheld the report, and **spam** means the
/// report said so. A report that was dismissed does not count, and a report
/// that is still open does not count — because an open report is an
/// allegation and a demotion is not founded on an allegation.
pub async fn confirmed_spam_reports(
    db: &Database,
    account: &str,
    window_days: i64,
) -> Result<i64, sqlx::Error> {
    let cutoff = (time::OffsetDateTime::now_utc() - time::Duration::days(window_days))
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let sql = db.sql(
        "SELECT COUNT(DISTINCT r.id)
           FROM reports r
           JOIN review_tasks t ON t.report_id = r.id
          WHERE r.subject_type = 'account' AND r.subject_id = ?
            AND LOWER(r.reason) = 'spam'
            AND LOWER(t.outcome) = 'uphold'
            AND r.created_at >= ?",
        "SELECT COUNT(DISTINCT r.id)
           FROM reports r
           JOIN review_tasks t ON t.report_id = r.id
          WHERE r.subject_type = 'account' AND r.subject_id = $1::uuid
            AND LOWER(r.reason) = 'spam'
            AND LOWER(t.outcome) = 'uphold'
            AND r.created_at >= $2",
    );
    let (count,): (i64,) = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .bind(&cutoff)
                .fetch_one(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .bind(&cutoff)
                .fetch_one(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(count)
}

/// Apply the one automatic demotion, if it is owed.
///
/// **Three confirmed spam reports in ninety days drops one level, never below
/// level 2** (spec §14.4). And it is the **only** automatic demotion: every
/// other change to a level is either earned from evidence or made by a person
/// who reviewed the conduct. A machine demoting a person is a machine
/// deciding what a person is worth.
///
/// **The floor is the specification's and it is not configurable.** A demotion
/// that could take an account to level 0 would make the account a guest again,
/// and a guest's contents are foldable away.
pub async fn apply_spam_demotion(
    db: &Database,
    account: &str,
    settings: &TrustSettings<'_>,
) -> Result<Option<i64>, sqlx::Error> {
    let confirmed = confirmed_spam_reports(
        db,
        account,
        darkhold_domain::trust::SPAM_DEMOTION_WINDOW_DAYS,
    )
    .await?;
    if !darkhold_domain::trust::spam_demotes(confirmed) {
        return Ok(None);
    }

    let current = trust_for(db, account).await?;
    let floor = settings.policy.floor_level.max(2);
    let demoted = darkhold_domain::trust::spam_demotion_level(current, floor);
    if demoted >= current {
        // The floor held: the account is already at it.
        return Ok(None);
    }

    write_trust(
        db,
        account,
        demoted,
        &serde_json::json!({
            "cause": "demotion_spam",
            "confirmed_spam_reports": confirmed,
            "window_days": darkhold_domain::trust::SPAM_DEMOTION_WINDOW_DAYS,
            "previous_level": current,
        })
        .to_string(),
    )
    .await?;
    crate::trust_events::record(
        db,
        account,
        current,
        demoted,
        crate::trust_events::TrustCause::DemotionSpam,
        Some(&confirmed.to_string()),
        None,
    )
    .await?;
    Ok(Some(demoted))
}

/// The accounts holding a privilege, by level.
///
/// **The pool is counted from the authority** — `trust_levels`, which is the
/// table the level and its basis live in — and not from the cache on
/// `accounts`. The two hold the same number and one function writes both, but
/// the authority is the one that cannot drift, because it is the one the
/// level was *computed* into.
pub async fn holder_counts(
    db: &Database,
) -> Result<darkhold_domain::trust::HolderCounts, sqlx::Error> {
    // **Every account, by level** — not only the ones who hold a particular
    // privilege. The question the elastic rule asks is "how many people could
    // hold this?", and the answer is a distribution of *people*: the level a
    // requirement falls to is the level at which enough of them are found.
    //
    // The count comes from `accounts.trust_level`, which every account row has
    // and which `write_trust` keeps equal to the authority in `trust_levels`.
    // The distribution of people is a fact about the accounts table, and
    // asking it of the accounts table is what makes it complete: a table of
    // accounts whose trust has been computed would be missing the accounts
    // whose trust has not.
    let sql = db.sql(
        "SELECT trust_level, COUNT(*) FROM accounts GROUP BY trust_level",
        "SELECT CAST(trust_level AS BIGINT), COUNT(*) FROM accounts GROUP BY trust_level",
    );
    let rows: Vec<(i64, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    let mut counts = darkhold_domain::trust::HolderCounts::default();
    for (level, count) in rows {
        *counts.by_level.entry(level).or_insert(0) += count;
    }
    Ok(counts)
}

/// The effective requirement for a privilege, right now.
///
/// **The stored one when there is one**, and the elastic one computed from the
/// current pool when there is not. An instance that has never evaluated its
/// requirements — a fresh one, or one whose trust has not been recomputed
/// since it started — must still answer the question correctly, so the
/// fallback computes rather than guessing.
pub async fn effective_requirement(
    db: &Database,
    ladder: &darkhold_domain::trust::TrustLadder,
    requirements: &darkhold_domain::trust::TrustRequirements,
    policy: &darkhold_domain::trust::ElasticPolicy,
    privilege: darkhold_domain::trust::Privilege,
) -> Result<StoredRequirement, sqlx::Error> {
    if let Some(stored) = requirement_for(db, privilege).await? {
        return Ok(stored);
    }
    let nominal = requirements.level_for(ladder, privilege);
    if nominal == i64::MAX {
        // The ladder grants it to nobody, so nobody holds it. An empty pool is
        // a fact, not an error.
        return Ok(StoredRequirement {
            level: nominal,
            basis: darkhold_domain::trust::RequirementBasis::Nominal,
            holders: 0,
        });
    }
    let holders = holder_counts(db).await?;
    let requirement = darkhold_domain::trust::elastic_requirement(nominal, &holders, policy);
    Ok(StoredRequirement {
        level: requirement.level,
        basis: requirement.basis,
        holders: holders.at_or_above(nominal),
    })
}

/// A privilege's stored requirement.
#[derive(Debug, Clone)]
pub struct StoredRequirement {
    /// The level the privilege requires.
    pub level: i64,
    /// Whether it is the ladder's own or fell because nobody held it.
    pub basis: darkhold_domain::trust::RequirementBasis,
    /// How many accounts held it when the requirement was evaluated.
    pub holders: i64,
}

/// Evaluate every privilege's requirement and store it.
///
/// **This is "the next evaluation"** (spec §14.4). It runs when a trust level
/// changes — which is the only time the pool can have changed — and it is a
/// pure function of the pool, so it cannot ratchet.
///
/// The evaluation is written **before** the events, so a reader who asks "why
/// is the queue open" gets the answer and not a mystery.
pub async fn evaluate_requirements(
    db: &Database,
    ladder: &darkhold_domain::trust::TrustLadder,
    requirements: &darkhold_domain::trust::TrustRequirements,
    policy: &darkhold_domain::trust::ElasticPolicy,
) -> Result<Vec<(darkhold_domain::trust::Privilege, StoredRequirement)>, sqlx::Error> {
    // **The distribution is fetched once**, not once per privilege: it is the
    // same table every time, and twenty-six queries to count one thing is a
    // cost nobody should pay on a trust change.
    let holders = holder_counts(db).await?;
    let mut decided = Vec::new();
    for privilege in darkhold_domain::trust::Privilege::all() {
        // The instance-wide settings are the operator role's, not the ladder's,
        // so they have no requirement to evaluate.
        if privilege == darkhold_domain::trust::Privilege::InstanceSettings {
            continue;
        }
        let nominal = requirements.level_for(ladder, privilege);
        if nominal == i64::MAX {
            // The ladder grants it to nobody.
            continue;
        }
        let requirement = darkhold_domain::trust::elastic_requirement(nominal, &holders, policy);
        let previous = requirement_for(db, privilege).await?;
        store_requirement(db, privilege, requirement, holders.at_or_above(nominal)).await?;

        // **The elastic grant is recorded** (spec §6.7). A privilege that
        // nobody holds and that somebody therefore holds *by the elastic
        // allocation* is a fact about the instance, and the trust history is
        // where the instance's facts about trust live.
        //
        // **Only when the grant is new.** The evaluation runs on every trust
        // change, and an event per evaluation would bury the history under
        // repetitions of the same fact.
        if requirement.basis == darkhold_domain::trust::RequirementBasis::Elastic {
            let already = previous.as_ref().is_some_and(|stored| {
                stored.basis == requirement.basis && stored.level == requirement.level
            });
            if !already {
                for level in holders.by_level.keys().rev() {
                    if *level < requirement.level {
                        break;
                    }
                    record_elastic_grant(db, privilege, *level).await?;
                }
            }
        }

        decided.push((
            privilege,
            StoredRequirement {
                level: requirement.level,
                basis: requirement.basis,
                holders: holders.at_or_above(nominal),
            },
        ));
    }
    Ok(decided)
}

/// Record that the accounts at a level hold a privilege by elastic allocation.
///
/// **The event does not move the level.** It records what the account may do,
/// not what it is — which is why `from_level` and `to_level` are the same, and
/// why the cause is `elastic` rather than one of the four axes.
async fn record_elastic_grant(
    db: &Database,
    privilege: darkhold_domain::trust::Privilege,
    level: i64,
) -> Result<(), sqlx::Error> {
    let sql = db.sql(
        "SELECT account FROM trust_levels WHERE level = ?",
        "SELECT account::text FROM trust_levels WHERE level = $1",
    );
    let rows: Vec<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(level)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(level)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    for (account,) in rows {
        crate::trust_events::record(
            db,
            &account,
            level,
            level,
            crate::trust_events::TrustCause::Elastic,
            Some(privilege.as_str()),
            None,
        )
        .await?;
    }
    Ok(())
}

/// Store one privilege's requirement.
async fn store_requirement(
    db: &Database,
    privilege: darkhold_domain::trust::Privilege,
    requirement: darkhold_domain::trust::Requirement,
    holders: i64,
) -> Result<(), sqlx::Error> {
    let now = crate::identity::now_rfc3339();
    let basis = match requirement.basis {
        darkhold_domain::trust::RequirementBasis::Nominal => "nominal",
        darkhold_domain::trust::RequirementBasis::Elastic => "elastic",
    };
    let sql = db.sql(
        "INSERT INTO privilege_requirements (privilege, level, basis, holders, evaluated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(privilege) DO UPDATE SET level=excluded.level, basis=excluded.basis,
         holders=excluded.holders, evaluated_at=excluded.evaluated_at",
        "INSERT INTO privilege_requirements (privilege, level, basis, holders, evaluated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT(privilege) DO UPDATE SET level=excluded.level, basis=excluded.basis,
         holders=excluded.holders, evaluated_at=excluded.evaluated_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(privilege.as_str())
                .bind(requirement.level)
                .bind(basis)
                .bind(holders)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite handle"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(privilege.as_str())
                .bind(requirement.level as i32)
                .bind(basis)
                .bind(holders as i32)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres handle"))
                .await?;
        }
    }
    Ok(())
}

/// The stored requirement for a privilege.
///
/// **The stored one, not a recomputation.** The requirement is evaluated when
/// the pool changes and not on every request, because the pool is a count over
/// every account and a trust gate is asked on every request a reader makes
/// that they are not allowed to make.
pub async fn requirement_for(
    db: &Database,
    privilege: darkhold_domain::trust::Privilege,
) -> Result<Option<StoredRequirement>, sqlx::Error> {
    let sql = db.sql(
        "SELECT level, basis, holders FROM privilege_requirements WHERE privilege = ?",
        "SELECT CAST(level AS BIGINT), basis, CAST(holders AS BIGINT)
           FROM privilege_requirements WHERE privilege = $1",
    );
    let row: Option<(i64, String, i64)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(privilege.as_str())
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(privilege.as_str())
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(level, basis, holders)| StoredRequirement {
        level,
        basis: match basis.as_str() {
            "elastic" => darkhold_domain::trust::RequirementBasis::Elastic,
            _ => darkhold_domain::trust::RequirementBasis::Nominal,
        },
        holders,
    }))
}

/// Set an account's trust level and its basis.
///
/// **The one writer**, alongside [`write_trust`] — which it calls, so the
/// level table and the cache the tier gate reads cannot disagree. A second
/// writer for one number is how two numbers come to exist.
pub async fn set_trust(
    db: &Database,
    account: &str,
    level: i64,
    basis: &str,
) -> Result<(), sqlx::Error> {
    write_trust(db, account, level, basis).await
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

/// One audit entry.
///
/// **A struct rather than eight parameters**, because a call site that reads
/// `audit_append(db, &actor, &action, "report", &id, &document)` cannot be
/// read without counting the arguments, and the two the modlog needs most —
/// the acting pseudonym and the reason — are exactly the ones a positional
/// call site would lose.
#[derive(Debug, Clone, Copy)]
pub struct AuditEntry<'a> {
    /// The account that acted. The instance's own record.
    pub actor: &'a str,
    /// The pseudonym the account acted as.
    ///
    /// **The modlog's public face** (spec §14.6). The account is the
    /// instance's record and this is the reader's, and a reader who cannot
    /// link a pseudonym to an account cannot be traced through the modlog.
    ///
    /// `None` where there is no pseudonym behind the action — a scheduled
    /// job, an import — because a made-up pseudonym would be worse than an
    /// honest absence.
    pub actor_pseud: Option<&'a str>,
    /// Why the action was taken.
    ///
    /// **Empty when the action had no reason, never absent**: the modlog's
    /// contract says every entry carries a reason, and a contract that is
    /// sometimes met is not a contract.
    pub reason: Option<&'a str>,
    /// What was done.
    pub action: &'a str,
    /// What kind of thing it was done to.
    pub subject_type: &'a str,
    /// Which one.
    pub subject_id: &'a str,
    /// The action's detail.
    pub document: &'a str,
}

/// One entry in the modlog, as the public sees it (spec §14.6).
///
/// **The five fields the specification names**: actor, action, target, reason
/// and timestamp. And the actor is a **pseudonym**.
#[derive(Debug, Clone)]
pub struct ModlogEntry {
    /// The entry's id.
    pub id: String,
    /// The acting pseudonym, if the action had one.
    ///
    /// **`None` where there was no person behind the action** — a scheduled
    /// job, an import, a decision the instance made through its panel. An
    /// honest absence beats a made-up one, and a made-up one would be a lie
    /// in the one place a lie cannot be corrected.
    pub actor_pseud: Option<String>,
    /// The acting pseudonym's handle, so a reader can see who acted without
    /// the account behind it.
    pub actor_handle: Option<String>,
    /// Whether the action was taken by the instance rather than by a person.
    ///
    /// **A panel decision is the instance acting.** Spec §14.6 says
    /// metamoderation votes are anonymised in the modlog — "the actor is the
    /// jury panel, not the voter" — and a panel is not a person, so an action
    /// with no pseudonym behind it is the shape an anonymised vote takes.
    pub actor_is_panel: bool,
    /// What was done.
    pub action: String,
    /// The kind of thing it was done to.
    pub subject_type: String,
    /// Which one — **a pseudonym where the subject is an account**.
    ///
    /// A sanction's subject is an account, and publishing that account's id
    /// would publish exactly what every other public surface refuses to
    /// publish (spec §7, M2-07). So the subject is resolved to the account's
    /// public face.
    pub subject_id: String,
    /// Whether the subject was an account, and so was resolved to a pseudonym.
    pub subject_is_pseudonym: bool,
    /// Why it was done.
    pub reason: Option<String>,
    /// When it happened.
    pub created_at: String,
}

/// The modlog: the audit log's public face, newest first.
///
/// **The account is never published.** The audit log's `actor` column holds
/// the account that acted — right for an audit, which is the instance's own
/// record — but the modlog is a *public* surface, and publishing an account
/// id there would publish the very link M2-07 says no public surface may
/// publish.
///
/// The cursor is `(created_at, id)`, so two entries written in the same
/// millisecond do not make the second page overlap the first.
pub async fn list_modlog(
    db: &Database,
    limit: i64,
    cursor: Option<(&str, &str)>,
) -> Result<Vec<ModlogEntry>, sqlx::Error> {
    let (after_time, after_id) = cursor
        .map(|(t, i)| (t.to_owned(), i.to_owned()))
        .unwrap_or_default();
    let sql = db.sql(
        "SELECT id, actor_pseud, reason, action, subject_type, subject_id, created_at
           FROM audit_log
          WHERE (? = '' OR (created_at, id) < (?, ?))
          ORDER BY created_at DESC, id DESC LIMIT ?",
        "SELECT id::text, actor_pseud, reason, action, subject_type, subject_id, created_at
           FROM audit_log
          WHERE ($1 = '' OR (created_at, id) < ($2, $3::uuid))
          ORDER BY created_at DESC, id DESC LIMIT $4",
    );
    let rows: Vec<ModlogRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(after_time.as_str())
                .bind(after_time.as_str())
                .bind(after_id.as_str())
                .bind(limit)
                .fetch_all(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(after_time.as_str())
                .bind(after_time.as_str())
                .bind(after_id.as_str())
                .bind(limit)
                .fetch_all(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        entries.push(into_entry(row, db).await?);
    }
    Ok(entries)
}

/// The columns of a modlog row.
type ModlogRow = (
    String,         // id
    Option<String>, // actor_pseud
    Option<String>, // reason
    String,         // action
    String,         // subject_type
    String,         // subject_id
    String,         // created_at
);

/// Turn one modlog row into an entry, resolving the pseudonyms.
async fn into_entry(row: ModlogRow, db: &Database) -> Result<ModlogEntry, sqlx::Error> {
    {
        let (id, actor_pseud, reason, action, subject_type, subject_id, created_at) = row;
        let actor_handle = match &actor_pseud {
            Some(pseud) => pseud_handle(db, pseud).await?,
            None => None,
        };
        // A subject that is an account is published as that account's public
        // face — a pseudonym — and never as the account id.
        let (subject_id, subject_is_pseudonym) = if subject_type == "account" {
            match account_pseud(db, &subject_id).await? {
                Some(pseud) => (pseud, true),
                // An account that no longer exists has no public face, and its
                // id is not published either: the entry says "an account" and
                // stops there.
                None => (String::new(), true),
            }
        } else {
            (subject_id, false)
        };
        Ok(ModlogEntry {
            id,
            actor_pseud: actor_pseud.clone(),
            actor_handle,
            actor_is_panel: actor_pseud.is_none(),
            action,
            subject_type,
            subject_id,
            subject_is_pseudonym,
            reason,
            created_at,
        })
    }
}

/// A pseudonym's handle, if it still exists.
async fn pseud_handle(db: &Database, pseud: &str) -> Result<Option<String>, sqlx::Error> {
    let sql = db.sql(
        "SELECT handle FROM pseuds WHERE id = ?",
        "SELECT handle FROM pseuds WHERE id = ?::uuid",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(pseud)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(handle,)| handle))
}

/// The account's public face: its first pseudonym.
async fn account_pseud(db: &Database, account: &str) -> Result<Option<String>, sqlx::Error> {
    let sql = db.sql(
        "SELECT id FROM pseuds WHERE account_id = ? ORDER BY created_at, id LIMIT 1",
        "SELECT id::text FROM pseuds WHERE account_id = ?::uuid ORDER BY created_at, id LIMIT 1",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.sqlite_pool().expect("sqlite handle"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account)
                .fetch_optional(db.postgres_pool().expect("postgres handle"))
                .await?
        }
    };
    Ok(row.map(|(id,)| id))
}

/// Append one entry to the audit log.
///
/// **The one writer.** The modlog reads this table, and the modlog's
/// contract — actor, action, target, reason, timestamp — is why the actor and
/// the reason are columns and not something a caller can leave out.
pub async fn audit_append(db: &Database, entry: &AuditEntry<'_>) -> Result<(), sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(
                "INSERT INTO audit_log
                   (id, actor, actor_pseud, reason, action, subject_type, subject_id, document, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(entry.actor)
            .bind(entry.actor_pseud)
            .bind(entry.reason)
            .bind(entry.action)
            .bind(entry.subject_type)
            .bind(entry.subject_id)
            .bind(entry.document)
            .bind(&now)
            .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query(
                "INSERT INTO audit_log
                   (id, actor, actor_pseud, reason, action, subject_type, subject_id, document, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9)",
            )
            .bind(&id)
            .bind(entry.actor)
            .bind(entry.actor_pseud)
            .bind(entry.reason)
            .bind(entry.action)
            .bind(entry.subject_type)
            .bind(entry.subject_id)
            .bind(serde_json::from_str::<serde_json::Value>(entry.document).unwrap_or(serde_json::Value::Null))
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
