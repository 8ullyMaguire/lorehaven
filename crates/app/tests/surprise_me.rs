//! Audit item 7 — Surprise Me: one work from OUTSIDE the reader's taste profile.
//!
//! `surprise_me_work` in `crates/db/src/discovery.rs`. Spec §16.10: "reader explicitly
//! requests recommendations outside their profile. The system inverts usual weighting."
//!
//! ## What makes this different from `blind_date.rs`
//!
//! Blind Date's defining property is that it IGNORES the profile. Surprise Me's is that it
//! goes specifically AWAY from it, and that exclusion is the entire feature — so nearly
//! every test here is about the profile clause, which is the one thing Blind Date has no
//! test for because it has no such clause.
//!
//! The property Blind Date shares is determinism in (account, day): a surprise that changes
//! on every refresh is a slot machine, and a reader cannot point at a specific work to
//! bookmark, rate, or discuss it. That is tested here too, for the same reason.

use lorehaven_db::discovery::surprise_me_work;
use test_support::{scratch_dir, TestDb};

struct Fixture {
    tdb: TestDb,
    reader: String,
}

impl Fixture {
    async fn build(tag: &str) -> Self {
        let dir = scratch_dir(tag);
        let tdb = TestDb::connect_with_dir(tag, &dir).await;
        let reader = account(&tdb, &format!("{tag}-reader@example.com")).await;
        Self { tdb, reader }
    }

    /// A published, public, untagged work owned by a fresh author.
    async fn work(&self, title: &str) -> String {
        self.work_by(title, None).await
    }

    async fn work_by(&self, title: &str, tag_node: Option<&str>) -> String {
        let owner = account(
            &self.tdb,
            &format!("{}-{title}@example.com", uuid::Uuid::new_v4().simple()),
        )
        .await;
        let id = uuid::Uuid::new_v4().to_string();
        exec_with(
            &self.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, summary, lifecycle, visibility, \
                 published_at, created_at, updated_at, generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, ?4, \
                 'published', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
                 '2026-01-01T00:00:00Z', 'forbid')",
            &[
                &id,
                &owner,
                &title.to_string(),
                &format!("summary for {title}"),
            ],
        )
        .await;
        if let Some(node) = tag_node {
            self.tag_work(&id, node).await;
        }
        id
    }

    /// A taxonomy node, by canonical name. Freeform, since the profile keys on node id.
    async fn node(&self, kind: &str, canonical: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        // `norm` is NOT NULL and (kind, norm) is UNIQUE, so it has to be written, and the
        // canonical lowercased is what the rest of the codebase stores there.
        let norm = canonical.to_lowercase();
        exec_with(
            &self.tdb,
            "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) \
             VALUES (?1#u, ?2, ?3, ?4, '2026-01-01T00:00:00Z')",
            &[&id, &kind.to_string(), &canonical.to_string(), &norm],
        )
        .await;
        id
    }

    async fn tag_work(&self, work_id: &str, node_id: &str) {
        exec_with(
            &self.tdb,
            "INSERT INTO work_tags (work_id, node_id, weight, added_at) \
             VALUES (?1#u, ?2#u, 0, '2026-01-01T00:00:00Z')",
            &[&work_id.to_string(), &node_id.to_string()],
        )
        .await;
    }

    /// Write the reader's taste profile. `signals` is stored as JSON, and the two engines
    /// read it differently, so this writes it the way the reader's own row would be.
    async fn profile(&self, node_ids: &[String]) {
        let signals = serde_json::json!(node_ids);
        exec_with(
            &self.tdb,
            "INSERT INTO taste_profiles (account, signals, computed_at) \
             VALUES (?1#u, ?2, '2026-01-01T00:00:00Z')",
            &[&self.reader, &signals.to_string()],
        )
        .await;
    }

    async fn bookmark(&self, work_id: &str) {
        exec_with(
            &self.tdb,
            "INSERT INTO bookmarks (id, account_id, subject_id, subject_type, is_public, \
                 created_at, updated_at) \
             VALUES (?1#u, ?2#u, ?3#u, 'work', ?4#b, '2026-01-01T00:00:00Z', \
                 '2026-01-01T00:00:00Z')",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &self.reader,
                &work_id.to_string(),
                // `is_public` is INTEGER on SQLite and BOOLEAN on PostgreSQL, so this
                // goes through the `#b` marker. Two earlier attempts were wrong in the
                // same direction: the literal `1` (an integer) and the bound string
                // `'true'` (text). PostgreSQL coerces neither, and the fixture failed
                // there while SQLite accepted both.
                &"1".to_string(),
            ],
        )
        .await;
    }

    async fn pick(&self, day: &str) -> Option<String> {
        surprise_me_work(self.tdb.db(), &self.reader, day)
            .await
            .expect("surprise me")
            .candidate
            .map(|c| c.work_id)
    }
}

// ── the property that makes it Surprise Me and not Blind Date ───────────────

#[tokio::test]
async fn a_work_sharing_any_tag_with_the_profile_is_never_offered() {
    // THE clause. Everything else about this feature is bookkeeping.
    let f = Fixture::build("sm_exclude").await;
    let loved = f.node("fandom", "Star Wars").await;
    let stranger = f.node("fandom", "Cooking Drama").await;
    f.profile(std::slice::from_ref(&loved)).await;

    let in_profile = f.work_by("Matches the profile", Some(&loved)).await;
    let out_of_profile = f.work_by("Outside the profile", Some(&stranger)).await;
    // And one with NO tags at all: it cannot match the profile, so it is eligible.
    let untagged = f.work("No tags at all").await;

    for _ in 0..12 {
        let picked = f.pick("2026-10-02").await.expect("a surprise exists");
        assert_ne!(
            picked, in_profile,
            "a work carrying a profile tag was offered as a surprise"
        );
        assert!(
            picked == out_of_profile || picked == untagged,
            "the pick must come from the out-of-profile set"
        );
    }
}

#[tokio::test]
async fn one_shared_tag_is_enough_to_exclude() {
    // A work is not "in the profile" because it is MOSTLY outside it. §16.10 says outside
    // the profile, and a reader who reads one Star Wars novel does not want the surprise
    // slot to hand them another one, however unlike it is otherwise.
    let f = Fixture::build("sm_one_tag").await;
    let a = f.node("fandom", "Star Wars").await;
    let b = f.node("freeform", "space opera").await;
    f.profile(std::slice::from_ref(&a)).await;

    let mixed = f.work_by("Half in profile", Some(&a)).await;
    exec_with(
        &f.tdb,
        "INSERT INTO work_tags (work_id, node_id, weight, added_at) \
             VALUES (?1#u, ?2#u, 0, '2026-01-01T00:00:00Z')",
        &[&mixed, &b],
    )
    .await;
    let clean = f.work("Genuinely elsewhere").await;

    for _ in 0..10 {
        assert_ne!(f.pick("2026-10-02").await.unwrap(), mixed);
    }
    // And it must actually pick something, or the exclusion above proves nothing.
    assert!(f.pick("2026-10-02").await.is_some());
    let _ = clean;
}

#[tokio::test]
async fn a_reader_with_no_profile_is_offered_the_whole_catalogue() {
    // The empty-profile case, and it is the one a JOIN would silently get wrong: a reader
    // who has never rated anything has signals `{}`, and "outside the profile" is then the
    // entire catalogue. Reporting "nothing available" to a new reader is a different and
    // wrong claim — the instance has plenty, the reader just has no profile to go outside.
    let f = Fixture::build("sm_empty_profile").await;
    let only = f.work("The only work").await;

    let got = surprise_me_work(f.tdb.db(), &f.reader, "2026-10-02")
        .await
        .expect("query");
    let got = got
        .candidate
        .expect("a surprise exists for a reader with no profile");
    assert_eq!(got.work_id, only);
    assert!(
        got.profile_empty,
        "the caller must be able to tell 'no profile' from 'nothing eligible'"
    );
}

#[tokio::test]
async fn an_empty_profile_object_is_treated_as_no_profile() {
    // The same state written the way the application actually writes it: a row present
    // with an EMPTY object rather than no row at all. Both mean "no signals", and if the
    // `json_each` subquery returned a null for one of them the surface would report
    // nothing for a new reader.
    let f = Fixture::build("sm_empty_object").await;
    let only = f.work("The only work").await;
    f.profile(&[]).await;

    let got = surprise_me_work(f.tdb.db(), &f.reader, "2026-10-02")
        .await
        .expect("query");
    let got = got.candidate.expect("an empty profile is not an exclusion");
    assert_eq!(got.work_id, only);
    assert!(
        got.profile_empty,
        "an empty signals object is still an empty profile"
    );
}

// ── the clauses shared with every other discovery surface ────────────────────

#[tokio::test]
async fn a_bookmarked_work_is_never_offered() {
    let f = Fixture::build("sm_bookmark").await;
    let saved = f.work("Already saved").await;
    let other = f.work("Something else").await;
    f.bookmark(&saved).await;

    for _ in 0..10 {
        assert_ne!(f.pick("2026-10-02").await.unwrap(), saved);
    }
    assert_eq!(f.pick("2026-10-02").await, Some(other));
}

#[tokio::test]
async fn the_reader_sees_their_own_works() {
    // Deliberate, and the opposite of Blind Date, which excludes authors the reader has
    // finished. Surprise Me is about TASTE distance, and a reader who writes in a genre
    // they do not read has every right to be surprised by their own back catalogue. On a
    // small instance excluding it would return nothing at all.
    //
    // The reader's account already has a pseud from `Fixture::build`, so a work authored by
    // that pseud is genuinely theirs. My first version tried to insert the account a
    // second time to "give" them an author, and the primary key rejected it.
    let f = Fixture::build("sm_own_work").await;
    let mine = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, \
             published_at, created_at, updated_at, generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
             'published', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z', 'forbid')",
        &[&mine, &f.reader, &"Written by the reader".to_string()],
    )
    .await;

    // Nothing else exists, so the ONLY possible answer is their own work. If own works
    // were excluded, this returns None.
    assert_eq!(
        f.pick("2026-10-02").await,
        Some(mine),
        "the reader's own works must not be excluded from the surprise surface"
    );
}

// ── determinism, shared with Blind Date ─────────────────────────────────────

#[tokio::test]
async fn the_pick_is_stable_across_reloads_on_the_same_day() {
    let f = Fixture::build("sm_stable").await;
    for n in 0..20 {
        f.work(&format!("stable-{n}")).await;
    }
    let first = f.pick("2026-10-02").await.expect("a surprise");
    for _ in 0..8 {
        assert_eq!(
            f.pick("2026-10-02").await,
            Some(first.clone()),
            "eight reloads later the reader gets a different surprise, so there is nothing to bookmark"
        );
    }
}

/// The seed is per READER, so the pick is not a single daily work for everyone.
///
/// ## Why this asserts "not always identical" rather than "always different"
///
/// My first version asserted `seen.len() == 2` over 20 works and was flaky at 5% — one
/// failure per twenty runs, which is the worst kind of test to leave in a suite. The
/// arithmetic: two independent FNV-1a picks over 20 candidates agree about 1 time in 20,
/// and 1/20 is 5.15% measured over 2000 trials.
///
/// What the store actually promises is that the SEED differs per reader, not that the
/// ANSWER differs. A collision is correct behaviour — two readers can genuinely be
/// surprised by the same work — so an exact-equality assertion was testing a property the
/// code never claimed.
///
/// The property that IS load-bearing is that the seed is not constant: if every reader
/// shared one seed, the pick would be the same for everyone on a given day, which is the
/// trending slot §16.10 is trying not to be. So this test runs many readers and asserts
/// that they do NOT all get the same work, with enough readers that a constant seed could
/// not survive by luck.
#[tokio::test]
async fn the_pick_is_not_the_same_work_for_every_reader() {
    let f = Fixture::build("sm_diff_readers").await;
    for n in 0..20 {
        f.work(&format!("shared-{n}")).await;
    }

    let mut seen = std::collections::HashSet::new();
    let mut readers = Vec::new();
    for n in 0..12 {
        readers.push(account(&f.tdb, &format!("sm_reader_{n}@example.com")).await);
    }
    for reader in &readers {
        let got = surprise_me_work(f.tdb.db(), reader, "2026-10-02")
            .await
            .expect("query");
        let got = got.candidate.expect("a surprise");
        seen.insert(got.work_id);
    }

    // A constant seed would give exactly 1. Twelve readers agreeing on one of twenty works
    // by chance has probability 20 * (1/20)^12 — so this cannot pass by luck, and it
    // cannot fail spuriously either.
    assert!(
        seen.len() > 1,
        "twelve readers all got the same work, so the seed is not per-reader"
    );
    assert!(
        seen.len() <= readers.len().min(20),
        "the picks must come from the catalogue that exists"
    );
}

#[tokio::test]
async fn an_empty_catalogue_reports_nothing_rather_than_failing() {
    // The route needs an empty state. An error here would turn "no surprises today" into a
    // 500, which is the difference between a quiet surface and a broken one.
    let f = Fixture::build("sm_empty").await;
    assert!(f.pick("2026-10-02").await.is_none());
}

#[tokio::test]
async fn a_catalogue_entirely_inside_the_profile_reports_nothing() {
    // Every eligible work shares a profile tag, so there is no surprise to give. `None` is
    // the honest answer and the frontend has a message for it; an error would be a lie
    // about the server.
    let f = Fixture::build("sm_all_in_profile").await;
    let loved = f.node("fandom", "Only This").await;
    f.profile(std::slice::from_ref(&loved)).await;
    for n in 0..5 {
        f.work_by(&format!("all-in-{n}"), Some(&loved)).await;
    }
    assert!(f.pick("2026-10-02").await.is_none());
}

#[tokio::test]
async fn an_unpublished_or_unlisted_work_is_never_offered() {
    // A draft, and a published-but-unlisted work. `lifecycle` says whether a work is
    // published and `visibility` says whether it is listable, and a work can be one
    // without the other — so both clauses have to be tested independently.
    let f = Fixture::build("sm_eligibility").await;
    let draft = uuid::Uuid::new_v4().to_string();
    let owner = account(&f.tdb, "sm_elig_author@example.com").await;
    for (id, lifecycle, visibility) in [
        (&draft, "draft", "public"),
        (&uuid::Uuid::new_v4().to_string(), "published", "unlisted"),
    ] {
        exec_with(
            &f.tdb,
            "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, \
                 published_at, created_at, updated_at, generated_content_posture) \
             VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, ?4, ?5, \
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', \
                 'forbid')",
            &[
                id,
                &owner,
                &"ineligible".to_string(),
                &lifecycle.to_string(),
                &visibility.to_string(),
            ],
        )
        .await;
    }

    assert!(
        f.pick("2026-10-02").await.is_none(),
        "a draft or an unlisted work was offered as a surprise"
    );
}

#[tokio::test]
async fn a_work_scheduled_for_the_future_is_not_offered() {
    let f = Fixture::build("sm_future").await;
    let owner = account(&f.tdb, "sm_future_author@example.com").await;
    let id = uuid::Uuid::new_v4().to_string();
    exec_with(
        &f.tdb,
        "INSERT INTO works (id, owner_pseud_id, title, lifecycle, visibility, \
             published_at, created_at, updated_at, generated_content_posture) \
         VALUES (?1#u, (SELECT id FROM pseuds WHERE account_id = ?2#u), ?3, \
             'published', 'public', '2026-12-01T00:00:00Z', '2026-01-01T00:00:00Z', \
             '2026-01-01T00:00:00Z', 'forbid')",
        &[&id, &owner, &"not yet".to_string()],
    )
    .await;

    assert!(
        f.pick("2026-10-02").await.is_none(),
        "a work scheduled for December was offered in October"
    );
}

#[tokio::test]
async fn the_candidate_carries_the_title_and_summary_the_card_renders() {
    // The store returns the fields, so the route does not have to fetch the work again.
    // A surface that renders a bare id would make the reader click through to learn a
    // title, which is not a surprise, it is a search result.
    let f = Fixture::build("sm_fields").await;
    let w = f.work("A Title Worth Reading").await;
    let got = surprise_me_work(f.tdb.db(), &f.reader, "2026-10-02")
        .await
        .expect("query");
    let got = got.candidate.expect("a surprise");
    assert_eq!(got.work_id, w);
    assert_eq!(got.title, "A Title Worth Reading");
    assert!(!got.summary.is_empty(), "the summary comes back empty");
}

// ── fixtures ────────────────────────────────────────────────────────────────

async fn account(tdb: &TestDb, email: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec_with(
        tdb,
        "INSERT INTO accounts (id, email, created_at, updated_at) \
         VALUES (?1#u, ?2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[&id, &email.to_string()],
    )
    .await;
    exec_with(
        tdb,
        "INSERT INTO pseuds (id, account_id, handle, display_name, created_at, updated_at) \
         VALUES (?1#u, ?2#u, ?3, ?3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        &[
            &uuid::Uuid::new_v4().to_string(),
            &id.clone(),
            &format!("s{}", &uuid::Uuid::new_v4().to_string()[..8]),
        ],
    )
    .await;
    id
}

/// `?N#u` binds a native uuid on PostgreSQL and text on SQLite, `?N#i` a bigint, and
/// `?N#b` a boolean.
///
/// THREE markers, not two, and the third exists because of `bookmarks.is_public`. It is
/// INTEGER on SQLite and BOOLEAN on PostgreSQL, and with only `#u` and `#i` there was no
/// way to write it: the literal `1` gives 42804 "boolean but expression is of type
/// integer", and binding the string `'true'` gives the same code with "text". Both were
/// green on SQLite.
///
/// The PostgreSQL bind follows the MARKER in the template, not the value of the argument.
/// Keying on the value is wrong and was wrong here: `work_tags.weight` is bigint and its
/// argument is the string "0", so a value-keyed rule that saw "0" would bind a boolean.
async fn exec_with(tdb: &TestDb, tmpl: &str, args: &[&String]) {
    let db = tdb.db();
    match db.backend() {
        lorehaven_db::Backend::Sqlite => {
            let sql = tmpl.replace("#u", "").replace("#i", "").replace("#b", "");
            let mut q = sqlx::query(&sql);
            for a in args {
                q = q.bind(a.as_str());
            }
            q.execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("fixture insert");
        }
        lorehaven_db::Backend::Postgres => {
            let pg = (1..=8).fold(tmpl.to_string(), |acc, n| {
                acc.replace(&format!("?{n}#b"), &format!("${n}::boolean"))
                    .replace(&format!("?{n}#u"), &format!("${n}::uuid"))
                    .replace(&format!("?{n}#i"), &format!("${n}::bigint"))
                    .replace(&format!("?{n}"), &format!("${n}"))
            });
            let mut q = sqlx::query(&pg);
            for (i, a) in args.iter().enumerate() {
                let n = i + 1;
                if tmpl.contains(format!("?{n}#b").as_str()) {
                    q = q.bind(*a == "1");
                } else if uuid::Uuid::parse_str(a).is_ok() {
                    q = q.bind(uuid::Uuid::parse_str(a).expect("re-parsed above"));
                } else {
                    q = q.bind(*a);
                }
            }
            q.execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("fixture insert");
        }
    }
}

#[tokio::test]
async fn an_empty_catalogue_still_reports_the_readers_profile_state() {
    // The defect the Playwright journey found. `surprise_me_work` returned
    // `Option<SurpriseCandidate>`, and an `Option` cannot carry `profile_empty` when there is
    // no candidate — so the route filled in `false`. The result: a brand-new reader on an
    // instance with nothing published was told "everything here shares a tag with your
    // profile", which is false in two separate ways. They have no profile, and there is
    // nothing here.
    //
    // No Rust test caught this, because every existing one either had a candidate to assert
    // the flag on or never looked at the empty case at all. The flag is a property of the
    // READER, so it has to be answerable with no works in the catalogue — that is the whole
    // reason the return type is `SurprisePick` and not an `Option`.
    let f = Fixture::build("sm_empty_catalogue").await;

    let pick = surprise_me_work(f.tdb.db(), &f.reader, "2026-10-02")
        .await
        .expect("query");

    assert!(
        pick.candidate.is_none(),
        "precondition: the fixture publishes nothing, so there is no candidate. Without this          the test would pass on a catalogue that had a work in it"
    );
    assert!(
        pick.profile_empty,
        "a reader with no profile row and an empty catalogue is an EMPTY PROFILE — reporting \
         `false` here is what made the UI claim their profile covered everything here"
    );
}

#[tokio::test]
async fn an_empty_catalogue_still_reports_a_populated_profile_as_not_empty() {
    // The mirror of the test above, and the one that matters more: if `profile_empty` were
    // simply hardcoded to `true` in the empty branch, the test above would pass and this one
    // would fail. Together they say the flag is computed, not guessed.
    let f = Fixture::build("sm_empty_catalogue_populated").await;
    f.profile(&["cozy-mystery".to_string()]).await;

    let pick = surprise_me_work(f.tdb.db(), &f.reader, "2026-10-02")
        .await
        .expect("query");

    assert!(
        pick.candidate.is_none(),
        "precondition: still no published work"
    );
    assert!(
        !pick.profile_empty,
        "a reader WITH signals keeps a populated profile even when there is nothing to serve; \
         an empty catalogue says nothing about what the reader likes"
    );
}
