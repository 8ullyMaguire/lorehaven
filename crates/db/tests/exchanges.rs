//! M45-27 — gift-exchange terms and the pairing, on both engines.
//!
//! The matcher's arithmetic is unit-tested in `darkhold-domain`; what this file pins is what
//! only exists once a database is involved:
//!
//! 1. **Terms are whole, not appended** — a participant who removes a fandom must not keep it.
//! 2. **Terms from a non-participant are refused** — a stored row the matcher never reads is a
//!    person who believes they are in an exchange they are not in.
//! 3. **A self-exclusion is refused**, not silently dropped.
//! 4. **Values are normalised on the way in**, so `Dragon  Age` and `dragon age` are one fandom.
//! 5. **The pairing is maximum** — everybody joined is paired, and nobody is paired with
//!    themselves.
//! 6. **Exclusions are honoured in both directions.**
//! 7. **An impossible pairing writes nothing at all** — not even the pairs it could have made.
//! 8. **A second match is refused, naming when the first was taken.**
//! 9. **Reveal is idempotent** — the second call reports the first timestamp.
//!
//! Run on SQLite by default; set `DARKHOLD_TEST_PG_URL` for the PostgreSQL arm.

use std::time::Duration;

use darkhold_db::exchanges::{
    assignments, mine, reveal, run_match, save_terms, summary, terms, MatchOutcome, Reveal,
    SaveTerms,
};
use darkhold_db::Database;

fn make_config(url: String) -> darkhold_db::DatabaseConfig {
    darkhold_db::DatabaseConfig {
        url,
        max_connections: 2,
        acquire_timeout: Duration::from_secs(5),
        slow_query_warn: Duration::ZERO,
    }
}

async fn connect(tag: &str) -> Database {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "darkhold-exchanges-{}-{}-{}",
        tag,
        uuid::Uuid::new_v4(),
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let url = match std::env::var("DARKHOLD_TEST_PG_URL") {
        Ok(admin) => {
            let name = format!(
                "lh_exchanges_{}_{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            );
            let admin_db = Database::connect(&make_config(admin.clone()))
                .await
                .expect("connect to the admin database");
            sqlx::query(&format!("CREATE DATABASE {name}"))
                .execute(admin_db.postgres_pool().expect("postgres pool"))
                .await
                .expect("create a scratch database");
            admin_db.close().await;
            let (prefix, _) = admin
                .rsplit_once('/')
                .expect("the admin URL ends in a database");
            format!("{prefix}/{name}")
        }
        Err(_) => format!("sqlite://{}/darkhold.sqlite?mode=rwc", dir.display()),
    };
    let db = Database::connect(&make_config(url)).await.expect("connect");
    db.migrate().await.expect("migrate");
    db
}

fn pseud(tag: &str) -> String {
    format!("pseud-{tag}")
}

/// An event with `members` joined, created by `organiser`. Returns the event id.
async fn event_with(db: &Database, organiser: &str, members: &[&str]) -> String {
    let id = darkhold_db::events::create_event(db, "Yuletide 2026", "a gift exchange", organiser)
        .await
        .expect("create event");
    for member in members {
        darkhold_db::events::join_event(db, &id, member)
            .await
            .expect("join event");
    }
    id
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

#[tokio::test]
async fn terms_are_replaced_wholesale() {
    let db = connect("whole").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a")]).await;
    let a = pseud("a");

    save_terms(&db, &event, &a, &strings(&["dragon"]), &strings(&["space"]), &[])
        .await
        .expect("save");
    save_terms(&db, &event, &a, &strings(&["noir"]), &strings(&["space"]), &[])
        .await
        .expect("save again");

    let stored = terms(&db, &event).await.expect("terms");
    let mine = stored.iter().find(|t| t.account == a).expect("one row set");
    assert_eq!(mine.offers, vec!["noir".to_string()], "removed fandom came back");
    assert_eq!(mine.wants, vec!["space".to_string()]);
}

#[tokio::test]
async fn terms_from_a_non_participant_are_refused() {
    let db = connect("outsider").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a")]).await;

    let outcome = save_terms(&db, &event, &pseud("stranger"), &strings(&["x"]), &[], &[])
        .await
        .expect("no database error");
    assert_eq!(outcome, SaveTerms::NotAParticipant);
    assert!(
        terms(&db, &event).await.expect("terms").is_empty(),
        "nothing was stored for the stranger"
    );
}

#[tokio::test]
async fn a_self_exclusion_is_refused_rather_than_dropped() {
    let db = connect("selfex").await;
    let a = pseud("a");
    let event = event_with(&db, &pseud("org"), &[&a]).await;

    let outcome = save_terms(&db, &event, &a, &[], &[], &[a.clone()])
        .await
        .expect("no database error");
    assert_eq!(outcome, SaveTerms::SelfExclusion);
    assert!(terms(&db, &event).await.expect("terms").is_empty());
}

#[tokio::test]
async fn values_are_normalised_and_deduplicated() {
    let db = connect("norm").await;
    let a = pseud("a");
    let event = event_with(&db, &pseud("org"), &[&a]).await;

    save_terms(
        &db,
        &event,
        &a,
        &strings(&["  Dragon   Age ", "dragon age", "DRAGON AGE"]),
        &strings(&["Space Opera"]),
        &[],
    )
    .await
    .expect("save");

    let stored = terms(&db, &event).await.expect("terms");
    let mine = stored.iter().find(|t| t.account == a).expect("row set");
    assert_eq!(mine.offers, vec!["dragon age".to_string()], "{mine:?}");
    assert_eq!(mine.wants, vec!["space opera".to_string()], "{mine:?}");
}

#[tokio::test]
async fn everybody_joined_is_paired_and_nobody_with_themselves() {
    let db = connect("pair").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a"), &pseud("b"), &pseud("c")]).await;
    for member in ["a", "b", "c"] {
        save_terms(
            &db,
            &event,
            &pseud(member),
            &strings(&["dragon"]),
            &strings(&["space"]),
            &[],
        )
        .await
        .expect("save terms");
    }

    let outcome = run_match(&db, &event).await.expect("match");
    let MatchOutcome::Matched { assignments, .. } = outcome else {
        panic!("expected a pairing: {outcome:?}");
    };
    assert_eq!(assignments.len(), 3, "three participants, three gifts");
    for pair in &assignments {
        assert_ne!(pair.giver, pair.recipient, "self-pairing: {pair:?}");
        // The algorithm is recorded per row, so "why did this person get that one" is
        // answerable later; a value outside the CHECK vocabulary would fail the insert.
        assert!(
            ["forced", "augment", "improved"].contains(&pair.matched_by.as_str()),
            "unknown method: {}",
            pair.matched_by
        );
    }
    // The unique index is the invariant; a second giver for one recipient must fail there.
    let mut recipients: Vec<&str> = assignments.iter().map(|a| a.recipient.as_str()).collect();
    recipients.sort_unstable();
    recipients.dedup();
    assert_eq!(recipients.len(), 3, "one giver per recipient");
}

#[tokio::test]
async fn an_exclusion_is_honoured_in_both_directions() {
    let db = connect("exclude").await;
    let (a, b, c, d) = (pseud("a"), pseud("b"), pseud("c"), pseud("d"));
    let event = event_with(&db, &pseud("org"), &[&a, &b, &c, &d]).await;

    // `a` refuses `b`; the pairing must not contain either direction of that edge.
    save_terms(&db, &event, &a, &[], &[], &[b.clone()])
        .await
        .expect("save");

    let outcome = run_match(&db, &event).await.expect("match");
    let MatchOutcome::Matched { assignments, .. } = outcome else {
        panic!("four people can be paired: {outcome:?}");
    };
    assert_eq!(assignments.len(), 4);
    for pair in &assignments {
        assert!(
            !(pair.giver == a && pair.recipient == b) && !(pair.giver == b && pair.recipient == a),
            "the excluded edge was used: {pair:?}"
        );
    }
}

#[tokio::test]
async fn an_impossible_pairing_writes_nothing_naming_the_unmatched() {
    let db = connect("impossible").await;
    let (a, b, c) = (pseud("a"), pseud("b"), pseud("c"));
    let event = event_with(&db, &pseud("org"), &[&a, &b, &c]).await;
    // `c` refuses both others, and refuses them in the only direction there is.
    save_terms(&db, &event, &c, &[], &[], &[a.clone(), b.clone()])
        .await
        .expect("save");

    let outcome = run_match(&db, &event).await.expect("match");
    let MatchOutcome::Impossible { unmatched } = outcome else {
        panic!("this exchange cannot be paired: {outcome:?}");
    };
    assert!(unmatched.contains(&c), "{unmatched:?}");
    assert!(
        assignments(&db, &event).await.expect("assignments").is_empty(),
        "a refusal stores no rows, not even the pairs it could have made"
    );
}

#[tokio::test]
async fn a_second_match_is_refused_and_names_the_first() {
    let db = connect("once").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a"), &pseud("b")]).await;
    let first = run_match(&db, &event).await.expect("match");
    let MatchOutcome::Matched { assignments: rows, .. } = first else {
        panic!("two people can be paired");
    };
    let assigned_at = rows[0].assigned_at.clone();

    let second = run_match(&db, &event).await.expect("match again");
    assert_eq!(
        second,
        MatchOutcome::AlreadyMatched {
            assigned_at: assigned_at.clone()
        },
        "the pairing is the record"
    );
    assert_eq!(assignments(&db, &event).await.expect("rows").len(), 2);
}

#[tokio::test]
async fn reveal_is_idempotent_and_mine_answers_the_giver() {
    let db = connect("reveal").await;
    let (a, b) = (pseud("a"), pseud("b"));
    let event = event_with(&db, &pseud("org"), &[&a, &b]).await;
    run_match(&db, &event).await.expect("match");

    let before = mine(&db, &event, &a).await.expect("mine").expect("paired");
    assert!(before.recipient == b, "two people pair with each other");
    assert!(before.revealed_at.is_none(), "not revealed yet");

    let first = reveal(&db, &event).await.expect("reveal");
    let Reveal::Revealed { revealed_at } = first else {
        panic!("revealed: {first:?}");
    };
    // `now_rfc3339` is second-granular, so a re-stamped timestamp can be byte-identical to
    // the first and the mutation would be invisible. One second of real time is the only
    // thing that makes "the first timestamp" distinguishable from "a fresh one".
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    let second = reveal(&db, &event).await.expect("reveal again");
    assert_eq!(
        second,
        Reveal::Revealed {
            revealed_at: revealed_at.clone()
        },
        "the second reveal reports the first timestamp, not a new one"
    );
    let after = mine(&db, &event, &a).await.expect("mine").expect("paired");
    assert_eq!(after.revealed_at.as_deref(), Some(revealed_at.as_str()));
}

#[tokio::test]
async fn reveal_before_matching_is_refused_and_the_summary_counts() {
    let db = connect("summary").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a"), &pseud("b"), &pseud("c")]).await;

    assert_eq!(reveal(&db, &event).await.expect("reveal"), Reveal::NotMatched);
    let before = summary(&db, &event).await.expect("summary");
    assert_eq!((before.participants, before.matched, before.revealed), (3, 0, false));

    run_match(&db, &event).await.expect("match");
    let after = summary(&db, &event).await.expect("summary");
    assert_eq!((after.participants, after.matched, after.revealed), (3, 3, false));
}

#[tokio::test]
async fn one_participant_is_too_few() {
    let db = connect("lonely").await;
    let event = event_with(&db, &pseud("org"), &[&pseud("a")]).await;
    assert_eq!(
        run_match(&db, &event).await.expect("match"),
        MatchOutcome::TooFew { participants: 1 }
    );
}

