//! Acceptance: rec blurbs as second summaries (spec §49.4, M45-35).
//!
//! Four clauses, and the tests that pin each:
//!
//! | clause | test |
//! |---|---|
//! | top-rated note, beside the summary, never instead | `a_quotable_note_appears_beside_the_summary`, `the_author_summary_survives_a_rec_blurb`, `the_top_rated_note_is_the_one_surfaced` |
//! | only consented notes are eligible | `a_note_without_consent_is_never_surfaced`, `consent_is_the_readers_own_and_revokable`, `a_private_note_is_never_surfaced_even_when_consented` |
//! | the excerpt is bounded | `a_long_note_is_excerpted_and_never_shown_whole` |
//! | not a ranking input | `a_rec_blurb_does_not_change_the_metrics` |
//!
//! The store is unit-tested in `crates/db/src/rec_blurbs.rs`; this drives the
//! public work page, because §49.4's first clause is about what a READER sees.

use axum::http::StatusCode;
use lorehaven_app::config::Config;
use lorehaven_app::server;
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, Database};
use serde_json::Value;
use test_support::{id, scratch_dir, TestClient, TestDb};

/// Fixed timestamps, so two rows written in one test compare equal.
const NOW: &str = "2026-01-01 00:00:00";

fn config_for(dir: &std::path::Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = match std::env::var("LOREHAVEN_TEST_PG_URL") {
        Ok(url) => lorehaven_db::DatabaseConfig::new(url),
        Err(_) => lorehaven_db::DatabaseConfig::new(format!(
            "sqlite://{}/lorehaven.sqlite?mode=rwc",
            dir.display()
        )),
    };
    // Process-global buckets at 127.0.0.1: neighbouring suites exhaust the
    // development defaults long before this file finishes.
    config.rate_limits.auth = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.write = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config.rate_limits.default = lorehaven_app::limiter::Quota {
        burst: 1000,
        per_minute: 6000,
    };
    config
}

struct Harness {
    tdb: TestDb,
    config: Config,
    db: Database,
}

/// One reader: their account id and their pseud id.
struct Reader {
    account: String,
    pseud: String,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let config = config_for(&dir);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let db = tdb.db().clone();
        Self { tdb, config, db }
    }

    fn client(&self) -> TestClient {
        TestClient::new(server::build_router(AppState::new(
            self.config.clone(),
            self.db.clone(),
        )))
    }

    /// A signed-in reader.
    ///
    /// `sign_in_as` registers, and registration already creates the account's
    /// default pseud under this handle. Inserting another would collide on
    /// `pseuds_handle_normalized` — which reads like a product bug and is a
    /// fixture bug, so the existing pseud is fetched instead.
    async fn reader(&self, handle: &str) -> Reader {
        let mut client = self.client();
        let email = format!("{handle}@example.test");
        let account = test_support::sign_in_as(&mut client, &self.tdb, &email, handle).await;

        // Written twice rather than shared: `SqlitePool` and `PgPool` are
        // unrelated types, so one binding cannot hold both. PostgreSQL also
        // needs `id::text` because this is read as a String.
        let pseud: String = match self.db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar("SELECT id FROM pseuds WHERE account_id = ? LIMIT 1")
                    .bind(&account)
                    .fetch_one(self.db.sqlite_pool().expect("sqlite"))
                    .await
            }
            Backend::Postgres => {
                sqlx::query_scalar::<_, String>(
                    "SELECT id::text FROM pseuds WHERE account_id = $1::uuid LIMIT 1",
                )
                .bind(&account)
                .fetch_one(self.db.postgres_pool().expect("postgres"))
                .await
            }
        }
        .expect("the registered account should have a pseud");

        Reader { account, pseud }
    }

    /// A public published work owned by `owner`, with the author's own summary.
    async fn work(&self, owner: &Reader, title: &str) -> String {
        let work = id(&format!("m45-work-{title}"));
        let title = title.to_owned();
        self.exec(
            "INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at, published_at) \
             VALUES (?1, ?2, ?3, 'The author''s own summary.', 'public', 'published', ?4, ?4, ?4)",
            "INSERT INTO works (id, title, owner_pseud_id, summary, visibility, lifecycle, created_at, updated_at, published_at) \
             VALUES ($1::uuid, $2, $3::uuid, 'The author''s own summary.', 'public', 'published', $4, $4, $4)",
            &[&work, &title, &owner.pseud, NOW],
            None,
            None,
        )
        .await;
        work
    }

    /// A public rec note on a work, quotable only if `allow_quote`, and rated
    /// only if `stars` is given.
    async fn note(
        &self,
        writer: &Reader,
        work: &str,
        body: &str,
        allow_quote: bool,
        stars: Option<i64>,
    ) -> String {
        // `writer.pseud` cannot be interpolated directly: `format!` does not
        // support field access, so it is bound first.
        let writer_pseud = &writer.pseud;
        let review = id(&format!("m45-review-{work}-{writer_pseud}"));
        self.exec(
            "INSERT INTO review (id, account_id, pseud_id, work_id, body, is_public, published_at, created_at, updated_at, allow_quote) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6, ?6, ?7)",
            "INSERT INTO review (id, account_id, pseud_id, work_id, body, is_public, published_at, created_at, updated_at, allow_quote) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, TRUE, $6, $6, $6, $7)",
            &[
                &review,
                &writer.account,
                &writer.pseud,
                work,
                body,
                NOW,
                &if allow_quote { "1".to_owned() } else { "0".to_owned() },
            ],
            Some(6),
            None,
        )
        .await;

        if let Some(stars) = stars {
            // `rating_pseud_work` is UNIQUE over live rows, so one rating per
            // (pseud, work) is the most the schema allows.
            self.exec(
                "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                "INSERT INTO rating (id, account_id, pseud_id, work_id, stars, is_public, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5, TRUE, $6, $6)",
                &[
                    &id(&format!("m45-rating-{work}-{writer_pseud}")),
                    &writer.account,
                    &writer.pseud,
                    work,
                    &stars.to_string(),
                    NOW,
                ],
                None,
                // `stars` is the 5th of six parameters.
                Some(4),
            )
            .await;
        }
        review
    }

    /// Run a statement with `?N` placeholders, rewritten per engine.
    ///
    /// A numbered placeholder may REPEAT (`?4` twice for two timestamps) and it
    /// names *which* parameter supplies its value, not a position in a sequence.
    /// So parameters are indexed by the number, and the emitted bind order is
    /// each occurrence in statement order. Binding by occurrence order instead
    /// binds the WRONG VALUE to a repeated placeholder, and the symptom is a NOT
    /// NULL failure on a column the statement fills twice.
    /// `pg_bool_at` names the parameter index that is a BOOLEAN on PostgreSQL
    /// and INTEGER on SQLite; pass `None` when the statement binds none.
    /// `pg_int_at` is the same for a BIGINT column: SQLite's INTEGER is 8 bytes
    /// and accepts a string bind through sqlx's widening, but PostgreSQL's BIGINT
    /// rejects a text expression outright.
    async fn exec(
        &self,
        sqlite_sql: &str,
        pg_sql: &str,
        params: &[&str],
        pg_bool_at: Option<usize>,
        pg_int_at: Option<usize>,
    ) {
        // The query is built INSIDE each arm: `sqlx::Query` is monomorphic in its
        // database, so one binding cannot serve both pools.
        match self.db.backend() {
            Backend::Sqlite => {
                let (flat, _, order) = dialect(sqlite_sql);
                // Bound to a local: `db.sql`-style helpers that take a `&str`
                // reject a `&` temporary, because the temporary dies at the end
                // of the statement rather than at the end of the expression.
                let mut q = sqlx::query(&flat);
                for n in &order {
                    q = q.bind(params[n - 1]);
                }
                q.execute(self.db.sqlite_pool().expect("sqlite"))
                    .await
                    .expect("exec sqlite");
            }
            Backend::Postgres => {
                // Every placeholder is `$n` and densely numbered from 1, and every
                // id bind carries `::uuid`. A leading `?` here would bind column 1
                // twice and PostgreSQL would report a foreign-key violation on a
                // parent row that exists.
                let mut q = sqlx::query(pg_sql);
                for (i, p) in params.iter().enumerate() {
                    // `allow_quote` is BOOLEAN on PostgreSQL and INTEGER on SQLite,
                    // so the same logical value cannot be bound as a string to
                    // both. The two arms therefore agree on the *parameter* and
                    // differ on how it is typed, which is the only honest way to
                    // write one fixture for two engines.
                    q = match (pg_bool_at == Some(i), pg_int_at == Some(i)) {
                        (true, _) => q.bind(*p == "1"),
                        (_, true) => q.bind(p.parse::<i64>().unwrap_or_default()),
                        _ => q.bind(*p),
                    };
                }
                q.execute(self.db.postgres_pool().expect("postgres"))
                    .await
                    .expect("exec postgres");
            }
        }
    }

    /// The public work page, as JSON.
    async fn work_page(&self, work: &str) -> Value {
        let mut client = self.client();
        let (status, body) = client.get(&format!("/api/v1/works/{work}")).await;
        assert_eq!(status, StatusCode::OK, "work page: {body}");
        body
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
    }
}

/// Rewrite `?N` placeholders per engine, reporting the bind order they imply.
///
/// `uuid_cols` names the columns whose PG type is `UUID` rather than `TEXT` --
/// which is every id column here, and is per-column and per-dialect rather than a
/// rule, so it is written out rather than inferred. Without the cast the PG arm
/// refuses an id bind with `column "id" is of type uuid but expression is of
/// type text`, while SQLite (whose ids are TEXT) accepts it.
/// Rewrite `?N` placeholders per engine, reporting the bind order they imply.
///
/// A numbered placeholder may REPEAT (`?4` twice for two timestamps) and it
/// names *which* parameter supplies its value, not a position in a sequence. So
/// parameters are indexed by the number, and the emitted bind order is each
/// occurrence in statement order. Binding by occurrence order instead binds the
/// WRONG VALUE to a repeated placeholder, and the symptom is a NOT NULL failure
/// on a column the statement fills twice.
///
/// The `::uuid` casts are written into the statement text at the call site rather
/// than derived here. That is deliberate: which columns are `UUID` is
/// per-column and per-dialect, not a rule -- `accounts.email` and `pseuds.handle`
/// are TEXT on BOTH engines -- so a helper that cast "every id-looking column"
/// would break those. Each call site therefore passes `cast`, which is applied to
/// the PostgreSQL arm only, because SQLite's ids are TEXT and `::uuid` is not a
/// token it accepts.
/// Rewrite `?N` placeholders per engine, reporting the bind order they imply.
///
/// A numbered placeholder may REPEAT (`?4` twice for two timestamps) and it
/// names *which* parameter supplies its value, not a position in a sequence. So
/// parameters are indexed by the number, and the emitted bind order is each
/// occurrence in statement order. Binding by occurrence order instead binds the
/// WRONG VALUE to a repeated placeholder, and the symptom is a NOT NULL failure
/// on a column the statement fills twice.
fn dialect(sql: &str) -> (String, String, Vec<usize>) {
    let mut sqlite = String::with_capacity(sql.len());
    let mut pg = String::with_capacity(sql.len());
    let mut order: Vec<usize> = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '?' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let n: usize = chars[i + 1..j].iter().collect::<String>().parse().unwrap();
            sqlite.push('?');
            // PostgreSQL numbers placeholders in STATEMENT order, not by the
            // number the test wrote, so a repeated `?4` becomes a fresh `$n`.
            pg.push('$');
            pg.push_str(&(order.len() + 1).to_string());
            order.push(n);
            i = j;
        } else {
            sqlite.push(chars[i]);
            pg.push(chars[i]);
            i += 1;
        }
    }
    (sqlite, pg, order)
}

// ---------------------------------------------------------------------------
// Clause 2: only consented notes are eligible
// ---------------------------------------------------------------------------

/// The clause that matters most: a note nobody consented to is not surfaced,
/// even when it is the highest-rated note on the work and the work is public.
/// The note here is deliberately the best candidate available — 5 stars, public,
/// published — so the ONLY reason it can be absent is the consent gate.
#[tokio::test]
async fn a_note_without_consent_is_never_surfaced() {
    let h = Harness::new("m45-no-consent").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Unconsented").await;
    h.note(
        &reader,
        &work,
        "Genuinely the best note here.",
        false,
        Some(5),
    )
    .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"],
        Value::Null,
        "a note that was never allowed to be quoted must not be surfaced: {page}"
    );

    h.cleanup().await;
}

/// Consent is the writer's own, readable, changeable, and revocable — and
/// revoking it takes the blurb away.
#[tokio::test]
async fn consent_is_the_readers_own_and_revokable() {
    let h = Harness::new("m45-revoke").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Revocable").await;
    let review = h
        .note(
            &reader,
            &work,
            "A note I am happy to be quoted on.",
            true,
            Some(4),
        )
        .await;

    assert_eq!(
        lorehaven_db::rec_blurbs::quote_consent(&h.db, &review)
            .await
            .expect("consent"),
        Some(true),
        "the writer must be able to read their own consent"
    );

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"]["pseud_handle"], "reader",
        "the consented note should be surfaced, attributed to its pseud: {page}"
    );

    assert!(
        lorehaven_db::rec_blurbs::set_quote_consent(&h.db, &review, false)
            .await
            .expect("withdraw"),
        "withdrawing consent on an existing note must report success"
    );
    assert_eq!(
        lorehaven_db::rec_blurbs::quote_consent(&h.db, &review)
            .await
            .expect("consent"),
        Some(false)
    );

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"],
        Value::Null,
        "withdrawing consent must remove the blurb, not merely mark it: {page}"
    );

    h.cleanup().await;
}

/// Consent to be quoted is not consent to publish. A reader may allow their note
/// to be a pull-quote while keeping the note itself private, and those are two
/// separate decisions they get to make.
#[tokio::test]
async fn a_private_note_is_never_surfaced_even_when_consented() {
    let h = Harness::new("m45-private-consented").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Private").await;
    let review = h
        .note(&reader, &work, "A private note.", true, Some(5))
        .await;

    h.exec(
        "UPDATE review SET is_public = 0 WHERE id = ?1",
        "UPDATE review SET is_public = FALSE WHERE id::text = $1",
        &[&review],
        None,
        None,
    )
    .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"],
        Value::Null,
        "consent to quote is not consent to publish: {page}"
    );

    h.cleanup().await;
}

// ---------------------------------------------------------------------------
// Clause 1: beside the summary, never instead of it
// ---------------------------------------------------------------------------

/// The blurb appears, with its attribution and its truncation flag.
#[tokio::test]
async fn a_quotable_note_appears_beside_the_summary() {
    let h = Harness::new("m45-beside").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Beside").await;
    h.note(
        &reader,
        &work,
        "The dialogue carries the whole plot.",
        true,
        Some(4),
    )
    .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"]["excerpt"], "The dialogue carries the whole plot.",
        "the consented note should be surfaced: {page}"
    );
    assert_eq!(
        page["rec_blurb"]["truncated"],
        Value::Bool(false),
        "a short note is not truncated"
    );
    assert_eq!(
        page["rec_blurb"]["pseud_handle"], "reader",
        "§49.4: always attributed"
    );

    h.cleanup().await;
}

/// The author's words are never displaced. §49.4 says "beside … never instead
/// of", and the only structural way to make that true is that the summary is a
/// field the blurb cannot overwrite.
#[tokio::test]
async fn the_author_summary_survives_a_rec_blurb() {
    let h = Harness::new("m45-survives").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Survives").await;
    h.note(&reader, &work, "A reader's take.", true, Some(5))
        .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["summary"], "The author's own summary.",
        "the author's summary must be present and unchanged alongside the blurb: {page}"
    );

    h.cleanup().await;
}

/// Clause 1's "top-rated": the highest star rating wins.
#[tokio::test]
async fn the_top_rated_note_is_the_one_surfaced() {
    let h = Harness::new("m45-top-rated").await;
    let author = h.reader("author").await;
    let low = h.reader("low").await;
    let high = h.reader("high").await;
    let work = h.work(&author, "TopRated").await;
    h.note(&low, &work, "A three-star note.", true, Some(3))
        .await;
    h.note(&high, &work, "A five-star note.", true, Some(5))
        .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"]["excerpt"], "A five-star note.",
        "the top-RATED note is the one surfaced: {page}"
    );
    assert_eq!(page["rec_blurb"]["stars"], Value::from(5));

    h.cleanup().await;
}

/// An unrated note is still a second summary. §49.4 says "the best
/// recommendation note", not "the best rated one", and an inner join would drop
/// exactly the unrated notes that most need a second summary.
#[tokio::test]
async fn an_unrated_note_is_still_eligible() {
    let h = Harness::new("m45-unrated").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Unrated").await;
    h.note(
        &reader,
        &work,
        "Rated by nobody, still worth showing.",
        true,
        None,
    )
    .await;

    let page = h.work_page(&work).await;
    assert_eq!(
        page["rec_blurb"]["excerpt"], "Rated by nobody, still worth showing.",
        "an unrated note is still a note: {page}"
    );
    assert_eq!(
        page["rec_blurb"]["stars"],
        Value::Null,
        "a missing rating is null, not zero and not a decode error"
    );

    h.cleanup().await;
}

// ---------------------------------------------------------------------------
// Clause 3: the excerpt is bounded
// ---------------------------------------------------------------------------

/// A long note is never shown whole.
#[tokio::test]
async fn a_long_note_is_excerpted_and_never_shown_whole() {
    let h = Harness::new("m45-bounded").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let work = h.work(&author, "Bounded").await;
    let long = "e".repeat(2000);
    h.note(&reader, &work, &long, true, Some(4)).await;

    let page = h.work_page(&work).await;
    let excerpt = page["rec_blurb"]["excerpt"]
        .as_str()
        .expect("an excerpt string");
    assert!(
        excerpt.chars().count() <= 240,
        "the excerpt must be bounded, got {} chars",
        excerpt.chars().count()
    );
    assert!(
        !excerpt.contains(&"e".repeat(241)),
        "the whole note leaked into the excerpt"
    );
    assert_eq!(
        page["rec_blurb"]["truncated"],
        Value::Bool(true),
        "a shortened pull must say it was shortened"
    );

    h.cleanup().await;
}

// ---------------------------------------------------------------------------
// Clause 4: rec blurbs are not a ranking input
// ---------------------------------------------------------------------------

/// A work with a surfaced rec blurb has the same metrics as one without.
///
/// §49.4 forbids rec blurbs being a ranking input, and §47.7's separation of
/// credit from ranking is why. The metrics block is the observable surface: a
/// blurb that moved any of it would be a blurb the ranker could see.
#[tokio::test]
async fn a_rec_blurb_does_not_change_the_metrics() {
    let h = Harness::new("m45-not-ranking").await;
    let author = h.reader("author").await;
    let reader = h.reader("reader").await;
    let with = h.work(&author, "WithBlurb").await;
    let without = h.work(&author, "WithoutBlurb").await;
    h.note(&reader, &with, "A note on the first work.", true, Some(5))
        .await;

    let page_with = h.work_page(&with).await;
    let page_without = h.work_page(&without).await;

    assert!(
        page_with["rec_blurb"].is_object(),
        "the first work should have a blurb"
    );
    assert!(page_without["rec_blurb"].is_null());

    // Only the REVIEW COUNT differs, and it differs because a review was written
    // on the first work and not the second — which is a pre-existing §9.4 counter
    // counting reviews, not the blurb leaking into ranking. Comparing the whole
    // block would assert that writing a review does not increment the review
    // count, which is a different (and wrong) claim.
    //
    // So the comparison is over every OTHER metric: those are the ones a ranker
    // could plausibly read a blurb through, and they must be identical.
    for key in [
        "bookmarks",
        "collection_adds",
        "complete_reads",
        "kudos",
        "reactions",
        "views",
    ] {
        assert_eq!(
            page_with["metrics"][key], page_without["metrics"][key],
            "a rec blurb must not move `{key}`, which a ranker reads: with={}, without={}",
            page_with["metrics"], page_without["metrics"]
        );
    }

    // And the blurb itself contributes nothing of its own: it is not a counter,
    // it is not a weight, and there is no field on the work page that grew
    // because a note was surfaced.
    assert_eq!(
        page_with["rec_blurb"]["stars"],
        Value::from(5),
        "the blurb carries its writer's stars as display data only"
    );

    h.cleanup().await;
}
