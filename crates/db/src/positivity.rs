use anyhow::Result;
use sqlx::FromRow;

use lorehaven_domain::positivity::{
    Classification, DeliveryOutcome, FeedbackClass, FeedbackPreferences, WorkFeedbackOverride,
};
use lorehaven_domain::{AccountId, PseudId, WorkId};

use crate::identity::now_rfc3339;
use crate::{Backend, Database};

/// A stored classification with its delivery outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClassification {
    pub class: FeedbackClass,
    pub confidence_bp: i64,
    pub signals: Vec<String>,
    pub outcome: DeliveryOutcome,
    pub classified_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct ClassificationRow {
    class: String,
    confidence_bp: i64,
    signals: String,
    outcome: String,
    classified_at: String,
}

impl ClassificationRow {
    fn decode(self) -> Option<StoredClassification> {
        let class = FeedbackClass::parse(&self.class)?;
        let outcome = match self.outcome.as_str() {
            "delivered" => DeliveryOutcome::Delivered,
            "held" => DeliveryOutcome::Held,
            _ => return None,
        };
        let signals: Vec<String> = serde_json::from_str(&self.signals).unwrap_or_default();
        Some(StoredClassification {
            class,
            confidence_bp: self.confidence_bp,
            signals,
            outcome,
            classified_at: self.classified_at,
        })
    }
}

#[derive(Debug, Clone, FromRow)]
struct PrefsRow {
    accept_constructive: i64,
    ambiguous_auto: i64,
    comments_enabled: i64,
}

#[derive(Debug, Clone, FromRow)]
struct OverrideRow {
    accept_constructive: Option<i64>,
    ambiguous_auto: Option<i64>,
    comments_enabled: Option<i64>,
}

fn decode_prefs(row: Option<PrefsRow>) -> FeedbackPreferences {
    match row {
        None => FeedbackPreferences::default(),
        Some(r) => FeedbackPreferences {
            accept_constructive: r.accept_constructive != 0,
            ambiguous_auto: r.ambiguous_auto != 0,
            comments_enabled: r.comments_enabled != 0,
        },
    }
}

fn decode_override(row: Option<OverrideRow>) -> WorkFeedbackOverride {
    match row {
        None => WorkFeedbackOverride::default(),
        Some(r) => WorkFeedbackOverride {
            accept_constructive: r.accept_constructive.map(|v| v != 0),
            ambiguous_auto: r.ambiguous_auto.map(|v| v != 0),
            comments_enabled: r.comments_enabled.map(|v| v != 0),
        },
    }
}

/// Account-level preferences, or the default when never set.
pub async fn preferences_for(db: &Database, account: AccountId) -> Result<FeedbackPreferences> {
    let sql = db.sql(
        "SELECT accept_constructive, ambiguous_auto_deliver AS ambiguous_auto, comments_enabled FROM feedback_preferences WHERE account_id = ?",
        "SELECT accept_constructive::int::bigint AS accept_constructive, ambiguous_auto_deliver::int::bigint AS ambiguous_auto, comments_enabled::int::bigint AS comments_enabled FROM feedback_preferences WHERE account_id::text = ?",
    );
    let row: Option<PrefsRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(account.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(decode_prefs(row))
}

/// Per-work overrides, or all-None (inherit) when never set.
pub async fn override_for(db: &Database, work: WorkId) -> Result<WorkFeedbackOverride> {
    let sql = db.sql(
        "SELECT accept_constructive, ambiguous_auto_deliver AS ambiguous_auto, comments_enabled FROM work_feedback_preferences WHERE work_id = ?",
        "SELECT accept_constructive::int::bigint AS accept_constructive, ambiguous_auto_deliver::int::bigint AS ambiguous_auto, comments_enabled::int::bigint AS comments_enabled FROM work_feedback_preferences WHERE work_id::text = ?",
    );
    let row: Option<OverrideRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(decode_override(row))
}

/// Version of the account preferences row, or 0 when never set.
pub async fn preferences_version(db: &Database, account: AccountId) -> Result<i64> {
    let sql = db.sql(
        "SELECT version FROM feedback_preferences WHERE account_id = ?",
        "SELECT version FROM feedback_preferences WHERE account_id::text = ?",
    );
    let v: Option<i64> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(account.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_scalar(&sql)
                .bind(account.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(v.unwrap_or(0))
}

/// Upsert the account default. `expected_version` is 0 for a first write.
/// Returns false on a stale write (nothing written).
pub async fn save_preferences(
    db: &Database,
    account: AccountId,
    prefs: &FeedbackPreferences,
    expected_version: i64,
) -> Result<bool> {
    let current = preferences_version(db, account).await?;
    if current != expected_version {
        return Ok(false);
    }
    let now = now_rfc3339();
    let constructive: i64 = i64::from(prefs.accept_constructive);
    let ambiguous: i64 = i64::from(prefs.ambiguous_auto);
    let enabled: i64 = i64::from(prefs.comments_enabled);
    let sql = db.sql(
        "INSERT INTO feedback_preferences (account_id, accept_constructive, ambiguous_auto_deliver, comments_enabled, author_note, created_at, updated_at, version)
         VALUES (?, ?, ?, ?, '', ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
              accept_constructive = excluded.accept_constructive,
              ambiguous_auto_deliver = excluded.ambiguous_auto_deliver,
              comments_enabled = excluded.comments_enabled,
              updated_at = excluded.updated_at,
              version = feedback_preferences.version + 1",
        "INSERT INTO feedback_preferences (account_id, accept_constructive, ambiguous_auto_deliver, comments_enabled, author_note, created_at, updated_at, version)
         VALUES (?::uuid, ?::int::boolean, ?::int::boolean, ?::int::boolean, '', ?, ?, 1)
         ON CONFLICT (account_id) DO UPDATE SET
              accept_constructive = excluded.accept_constructive,
              ambiguous_auto_deliver = excluded.ambiguous_auto_deliver,
              comments_enabled = excluded.comments_enabled,
              updated_at = excluded.updated_at,
              version = feedback_preferences.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(account.to_string())
                .bind(constructive)
                .bind(ambiguous)
                .bind(enabled)
                .bind(&now)
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(account.to_string())
                .bind(constructive)
                .bind(ambiguous)
                .bind(enabled)
                .bind(&now)
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(true)
}

/// Upsert a per-work override (all-None clears back to inherit).
pub async fn save_work_override(
    db: &Database,
    work: WorkId,
    ov: &WorkFeedbackOverride,
) -> Result<()> {
    let now = now_rfc3339();
    let to_int = |v: Option<bool>| v.map(i64::from);
    let sql = db.sql(
        "INSERT INTO work_feedback_preferences (work_id, accept_constructive, ambiguous_auto_deliver, comments_enabled, author_note, updated_at, version)
         VALUES (?, ?, ?, ?, NULL, ?, 1)
         ON CONFLICT (work_id) DO UPDATE SET
              accept_constructive = excluded.accept_constructive,
              ambiguous_auto_deliver = excluded.ambiguous_auto_deliver,
              comments_enabled = excluded.comments_enabled,
              updated_at = excluded.updated_at,
              version = work_feedback_preferences.version + 1",
        "INSERT INTO work_feedback_preferences (work_id, accept_constructive, ambiguous_auto_deliver, comments_enabled, author_note, updated_at, version)
         VALUES (?::uuid, ?::int::boolean, ?::int::boolean, ?::int::boolean, NULL, ?, 1)
         ON CONFLICT (work_id) DO UPDATE SET
              accept_constructive = excluded.accept_constructive,
              ambiguous_auto_deliver = excluded.ambiguous_auto_deliver,
              comments_enabled = excluded.comments_enabled,
              updated_at = excluded.updated_at,
              version = work_feedback_preferences.version + 1",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(work.to_string())
                .bind(to_int(ov.accept_constructive))
                .bind(to_int(ov.ambiguous_auto))
                .bind(to_int(ov.comments_enabled))
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(work.to_string())
                .bind(to_int(ov.accept_constructive))
                .bind(to_int(ov.ambiguous_auto))
                .bind(to_int(ov.comments_enabled))
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Whether `reviewer` is allow/denylisted by `author`.
pub async fn list_membership(
    db: &Database,
    author: AccountId,
    reviewer: PseudId,
) -> Result<(bool, bool)> {
    let allow_sql = db.sql(
        "SELECT 1 FROM feedback_allowlist WHERE author_account_id = ? AND trusted_pseud_id = ?",
        "SELECT 1::bigint FROM feedback_allowlist WHERE author_account_id::text = ? AND trusted_pseud_id::text = ?",
    );
    let deny_sql = db.sql(
        "SELECT 1 FROM feedback_denylist WHERE author_account_id = ? AND refused_pseud_id = ?",
        "SELECT 1::bigint FROM feedback_denylist WHERE author_account_id::text = ? AND refused_pseud_id::text = ?",
    );
    let (allow, deny): (Option<i64>, Option<i64>) = match db.backend() {
        Backend::Sqlite => {
            let pool = db.sqlite_pool().expect("sqlite");
            let a: Option<i64> = sqlx::query_scalar(&allow_sql)
                .bind(author.to_string())
                .bind(reviewer.to_string())
                .fetch_optional(pool)
                .await?;
            let d: Option<i64> = sqlx::query_scalar(&deny_sql)
                .bind(author.to_string())
                .bind(reviewer.to_string())
                .fetch_optional(pool)
                .await?;
            (a, d)
        }
        Backend::Postgres => {
            let pool = db.postgres_pool().expect("postgres");
            let a: Option<i64> = sqlx::query_scalar(&allow_sql)
                .bind(author.to_string())
                .bind(reviewer.to_string())
                .fetch_optional(pool)
                .await?;
            let d: Option<i64> = sqlx::query_scalar(&deny_sql)
                .bind(author.to_string())
                .bind(reviewer.to_string())
                .fetch_optional(pool)
                .await?;
            (a, d)
        }
    };
    Ok((allow.is_some(), deny.is_some()))
}

/// Record one classification plus its outcome. Upsert: re-submitting the
/// same review rewrites the row, never doubles it (idempotency).
pub async fn record_classification(
    db: &Database,
    review_id: &str,
    verdict: &Classification,
    outcome: DeliveryOutcome,
) -> Result<()> {
    let now = now_rfc3339();
    let signals = serde_json::to_string(&verdict.signals).unwrap_or_else(|_| "[]".to_owned());
    let sql = db.sql(
        "INSERT INTO review_classifications (review_id, class, confidence_bp, signals, outcome, classified_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (review_id) DO UPDATE SET class = excluded.class, confidence_bp = excluded.confidence_bp, signals = excluded.signals, outcome = excluded.outcome, classified_at = excluded.classified_at",
        "INSERT INTO review_classifications (review_id, class, confidence_bp, signals, outcome, classified_at)
         VALUES (?::uuid, ?, ?, ?, ?, ?)
         ON CONFLICT (review_id) DO UPDATE SET class = excluded.class, confidence_bp = excluded.confidence_bp, signals = excluded.signals, outcome = excluded.outcome, classified_at = excluded.classified_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(review_id)
                .bind(verdict.class.as_str())
                .bind(verdict.confidence_bp)
                .bind(&signals)
                .bind(outcome.as_str())
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(review_id)
                .bind(verdict.class.as_str())
                .bind(verdict.confidence_bp)
                .bind(&signals)
                .bind(outcome.as_str())
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Load one stored classification, if any.
pub async fn classification_for(
    db: &Database,
    review_id: &str,
) -> Result<Option<StoredClassification>> {
    let sql = db.sql(
        "SELECT class, confidence_bp, signals, outcome, classified_at FROM review_classifications WHERE review_id = ?",
        "SELECT class, confidence_bp, signals, outcome, classified_at FROM review_classifications WHERE review_id::text = ?",
    );
    let row: Option<ClassificationRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(review_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(review_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.and_then(ClassificationRow::decode))
}

/// Trust a pseud: their texts skip classification for this author.
pub async fn allow_pseud(db: &Database, author: AccountId, trusted: PseudId) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO feedback_allowlist (author_account_id, trusted_pseud_id, created_at) VALUES (?, ?, ?) ON CONFLICT (author_account_id, trusted_pseud_id) DO NOTHING",
        "INSERT INTO feedback_allowlist (author_account_id, trusted_pseud_id, created_at) VALUES (?::uuid, ?::uuid, ?) ON CONFLICT (author_account_id, trusted_pseud_id) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(author.to_string())
                .bind(trusted.to_string())
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(author.to_string())
                .bind(trusted.to_string())
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Refuse a pseud: their texts are always held for this author.
pub async fn deny_pseud(db: &Database, author: AccountId, refused: PseudId) -> Result<()> {
    let now = now_rfc3339();
    let sql = db.sql(
        "INSERT INTO feedback_denylist (author_account_id, refused_pseud_id, created_at) VALUES (?, ?, ?) ON CONFLICT (author_account_id, refused_pseud_id) DO NOTHING",
        "INSERT INTO feedback_denylist (author_account_id, refused_pseud_id, created_at) VALUES (?::uuid, ?::uuid, ?) ON CONFLICT (author_account_id, refused_pseud_id) DO NOTHING",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(author.to_string())
                .bind(refused.to_string())
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(author.to_string())
                .bind(refused.to_string())
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// The account behind a work's owner pseud. None when the work is gone.
pub async fn author_account_for_work(db: &Database, work: WorkId) -> Result<Option<AccountId>> {
    let sql = db.sql(
        "SELECT p.account_id FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id WHERE w.id = ? AND w.deleted_at IS NULL",
        "SELECT p.account_id::text AS account_id FROM works w JOIN pseuds p ON p.id = w.owner_pseud_id WHERE w.id::text = ? AND w.deleted_at IS NULL",
    );
    let row: Option<(String,)> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    match row {
        None => Ok(None),
        Some((id,)) => {
            let uuid: uuid::Uuid = id
                .parse()
                .map_err(|error| anyhow::anyhow!("bad account id: {error}"))?;
            Ok(Some(AccountId::from(uuid)))
        }
    }
}

/// Public reviews of a work with held text removed.
pub async fn visible_reviews(db: &Database, work: WorkId) -> Result<Vec<crate::reading::Review>> {
    let sql = db.sql(
        "SELECT r.id, p.handle AS author_handle, r.body, r.contains_spoilers, r.is_public, r.published_at, r.created_at, r.updated_at, r.version
           FROM review r JOIN pseuds p ON p.id = r.pseud_id
           LEFT JOIN review_classifications c ON c.review_id = r.id
          WHERE r.work_id = ? AND r.is_public = 1 AND r.published_at IS NOT NULL AND r.deleted_at IS NULL
            AND (c.outcome IS NULL OR c.outcome = 'delivered')
          ORDER BY r.published_at DESC, r.id ASC",
        "SELECT r.id::text AS id, p.handle AS author_handle, r.body, r.contains_spoilers::int::bigint, r.is_public::int::bigint, r.published_at, r.created_at, r.updated_at, r.version
           FROM review r JOIN pseuds p ON p.id = r.pseud_id
           LEFT JOIN review_classifications c ON c.review_id = r.id
          WHERE r.work_id::text = ? AND r.is_public = TRUE AND r.published_at IS NOT NULL AND r.deleted_at IS NULL
            AND (c.outcome IS NULL OR c.outcome = 'delivered')
          ORDER BY r.published_at DESC, r.id ASC",
    );
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        author_handle: String,
        body: String,
        contains_spoilers: i64,
        is_public: i64,
        published_at: Option<String>,
        created_at: String,
        updated_at: String,
        version: i64,
    }
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(work.to_string())
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|r| crate::reading::Review {
            id: r.id,
            author_handle: r.author_handle,
            body: r.body,
            contains_spoilers: r.contains_spoilers != 0,
            is_public: r.is_public != 0,
            published_at: r.published_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
            version: r.version,
        })
        .collect())
}

/// One delivered review on the author's own works, for the inbox.
#[derive(Debug, Clone)]
pub struct InboxItem {
    pub review_id: String,
    pub work_id: String,
    pub work_title: String,
    pub author_handle: String,
    pub body: String,
    pub class: String,
    pub published_at: Option<String>,
}

/// Delivered public reviews on works owned by this account's pseuds.
pub async fn inbox_for(db: &Database, author: AccountId) -> Result<Vec<InboxItem>> {
    let sql = db.sql(
        "SELECT r.id AS review_id, r.work_id AS work_id, w.title AS work_title, p.handle AS author_handle, r.body AS body, c.class AS class, r.published_at AS published_at
           FROM review r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds owner ON owner.id = w.owner_pseud_id
           JOIN pseuds p ON p.id = r.pseud_id
           LEFT JOIN review_classifications c ON c.review_id = r.id
          WHERE owner.account_id = ? AND w.deleted_at IS NULL AND r.is_public = 1 AND r.published_at IS NOT NULL AND r.deleted_at IS NULL
            AND (c.outcome IS NULL OR c.outcome = 'delivered')
          ORDER BY r.published_at DESC, r.id ASC LIMIT 100",
        "SELECT r.id::text AS review_id, r.work_id::text AS work_id, w.title AS work_title, p.handle AS author_handle, r.body AS body, c.class AS class, r.published_at AS published_at
           FROM review r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds owner ON owner.id = w.owner_pseud_id
           JOIN pseuds p ON p.id = r.pseud_id
           LEFT JOIN review_classifications c ON c.review_id = r.id
          WHERE owner.account_id::text = ? AND w.deleted_at IS NULL AND r.is_public = TRUE AND r.published_at IS NOT NULL AND r.deleted_at IS NULL
            AND (c.outcome IS NULL OR c.outcome = 'delivered')
          ORDER BY r.published_at DESC, r.id ASC LIMIT 100",
    );
    #[derive(sqlx::FromRow)]
    struct Row {
        review_id: String,
        work_id: String,
        work_title: String,
        author_handle: String,
        body: String,
        class: Option<String>,
        published_at: Option<String>,
    }
    let rows: Vec<Row> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(author.to_string())
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(author.to_string())
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|r| InboxItem {
            review_id: r.review_id,
            work_id: r.work_id,
            work_title: r.work_title,
            author_handle: r.author_handle,
            body: r.body,
            class: r.class.unwrap_or_else(|| "positive".to_owned()),
            published_at: r.published_at,
        })
        .collect())
}

/// How many held reviews sit on this author's works. A count, never content.
pub async fn held_count_for(db: &Database, author: AccountId) -> Result<i64> {
    let sql = db.sql(
        "SELECT COUNT(*) FROM review r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds owner ON owner.id = w.owner_pseud_id
           JOIN review_classifications c ON c.review_id = r.id
          WHERE owner.account_id = ? AND w.deleted_at IS NULL AND r.deleted_at IS NULL AND c.outcome = 'held'",
        "SELECT COUNT(*) FROM review r
           JOIN works w ON w.id = r.work_id
           JOIN pseuds owner ON owner.id = w.owner_pseud_id
           JOIN review_classifications c ON c.review_id = r.id
          WHERE owner.account_id::text = ? AND w.deleted_at IS NULL AND r.deleted_at IS NULL AND c.outcome = 'held'",
    );
    let n: i64 = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_scalar(&sql)
                .bind(author.to_string())
                .fetch_one(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            let n: i64 = sqlx::query_scalar(&sql)
                .bind(author.to_string())
                .fetch_one(db.postgres_pool().expect("postgres"))
                .await?;
            n
        }
    };
    Ok(n)
}

/// Classify and store the delivery outcome for one review write.
///
/// Pure classify + record_classification. The caller (route layer) handles
/// the compensating action if this fails: withdrawing the review so nothing
/// ungated survives. This keeps the repository function simple and testable.
pub async fn classify_review(
    db: &Database,
    review_id: &str,
    body: &str,
    prefs: &FeedbackPreferences,
    allow: bool,
    deny: bool,
) -> Result<StoredClassification> {
    use lorehaven_domain::positivity::{classify, resolve_delivery};
    let verdict = classify(body);
    let outcome = resolve_delivery(verdict.class, prefs, allow, deny);
    record_classification(db, review_id, &verdict, outcome).await?;
    let stored = classification_for(db, review_id)
        .await?
        .expect("just recorded");
    Ok(stored)
}

/// Classify and store the delivery outcome for one comment write.
pub async fn classify_comment(
    db: &Database,
    comment_id: &str,
    body: &str,
    prefs: &FeedbackPreferences,
    allow: bool,
    deny: bool,
) -> Result<StoredClassification> {
    use lorehaven_domain::positivity::{classify, resolve_delivery};
    let verdict = classify(body);
    let outcome = resolve_delivery(verdict.class, prefs, allow, deny);
    record_comment_classification(db, comment_id, &verdict, outcome).await?;
    let stored = comment_classification_for(db, comment_id)
        .await?
        .expect("just recorded");
    Ok(stored)
}

/// Record one comment classification plus its outcome.
pub async fn record_comment_classification(
    db: &Database,
    comment_id: &str,
    verdict: &Classification,
    outcome: DeliveryOutcome,
) -> Result<()> {
    let now = now_rfc3339();
    let signals = serde_json::to_string(&verdict.signals).unwrap_or_else(|_| "[]".to_owned());
    let sql = db.sql(
        "INSERT INTO comment_classifications (comment_id, class, confidence_bp, signals, outcome, classified_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (comment_id) DO UPDATE SET class = excluded.class, confidence_bp = excluded.confidence_bp, signals = excluded.signals, outcome = excluded.outcome, classified_at = excluded.classified_at",
        "INSERT INTO comment_classifications (comment_id, class, confidence_bp, signals, outcome, classified_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (comment_id) DO UPDATE SET class = excluded.class, confidence_bp = excluded.confidence_bp, signals = excluded.signals, outcome = excluded.outcome, classified_at = excluded.classified_at",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(comment_id)
                .bind(verdict.class.as_str())
                .bind(verdict.confidence_bp)
                .bind(&signals)
                .bind(outcome.as_str())
                .bind(&now)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(comment_id)
                .bind(verdict.class.as_str())
                .bind(verdict.confidence_bp)
                .bind(&signals)
                .bind(outcome.as_str())
                .bind(&now)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Load one stored comment classification, if any.
pub async fn comment_classification_for(
    db: &Database,
    comment_id: &str,
) -> Result<Option<StoredClassification>> {
    let sql = db.sql(
        "SELECT class, confidence_bp, signals, outcome, classified_at FROM comment_classifications WHERE comment_id = ?",
        "SELECT class, confidence_bp, signals, outcome, classified_at FROM comment_classifications WHERE comment_id = $1",
    );
    let row: Option<ClassificationRow> = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as(&sql)
                .bind(comment_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
        }
        Backend::Postgres => {
            sqlx::query_as(&sql)
                .bind(comment_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
        }
    };
    Ok(row.and_then(ClassificationRow::decode))
}
