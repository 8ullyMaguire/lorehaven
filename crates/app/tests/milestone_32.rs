//! M32 — Typed votes, vote budgets, meta-moderation, karma.
//!
//! Spec §35.2. These tests drive the real router against a real SQLite file and
//! prove the five things the spec's acceptance list names:
//!
//! - the budget is enforced per rolling window and per trust level, and
//!   exhaustion is *reported*, never silently dropped;
//! - a negative vote costs more budget than a positive one;
//! - meta-moderation moves a caster's future vote *weight*, never their ability
//!   to cast (weight, not voice, is the sanction);
//! - individual votes are not exposed where the transparency tier forbids it;
//! - karma follows received votes weighted at cast time, decays on inactivity,
//!   and is read by no code path that feeds trust, ranking or credits (a
//!   workspace grep, not a promise);
//! - a category's taxonomy is data: configuring it changes the surface with no
//!   code change.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::{Backend, DatabaseConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-m32-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn config_for(dir: &Path) -> Config {
    let mut config = Config::development_defaults();
    config.storage.root = dir.to_path_buf();
    config.database = DatabaseConfig::new(format!(
        "sqlite://{}/lorehaven.sqlite?mode=rwc",
        dir.display()
    ));
    config
}

struct Client {
    app: axum::Router,
    cookies: Vec<(String, String)>,
}

impl Client {
    fn new(app: axum::Router) -> Self {
        Self {
            app,
            cookies: Vec::new(),
        }
    }
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn capture(&mut self, response: &axum::response::Response) {
        for value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some((pair, _)) = text.split_once(';') else {
                continue;
            };
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_owned();
                let value = value.trim().to_owned();
                self.cookies.retain(|(k, _)| k != &name);
                if !value.is_empty() {
                    self.cookies.push((name, value));
                }
            }
        }
    }
    async fn request(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            builder = builder.header(
                header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
            if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
                builder = builder.header("x-csrf-token", token);
            }
        }
        let request = match body {
            Some(v) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&v).expect("serialise")))
                .expect("request"),
            None => builder.body(Body::empty()).expect("request"),
        };
        let response = self.app.clone().oneshot(request).await.expect("response");
        let status = response.status();
        self.capture(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .expect("body");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }
    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }
    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }
}

struct Harness {
    dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        Self::with_config(tag, |_| {}).await
    }

    /// A harness whose forum configuration the test has adjusted — the budget
    /// and decay numbers are configuration (spec §35.2), so tests set them
    /// rather than looping until a default runs out.
    async fn with_config(tag: &str, tweak: impl FnOnce(&mut Config)) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let mut config = config_for(&dir);
        tweak(&mut config);
        Self { dir, tdb, config }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }

    async fn cleanup(self) {
        self.tdb.cleanup().await;
        let _ = std::fs::remove_dir_all(self.dir);
    }
}

const PASSWORD: &str = "a-long-enough-passphrase";

async fn register(client: &mut Client, email: &str, handle: &str) -> (String, String) {
    let (status, body) = client
        .post(
            "/api/v1/auth/register",
            json!({
                "email": email,
                "password": PASSWORD,
                "handle": handle,
                "display_name": handle,
                "age_band": "adult"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
    let (status, me) = client.get("/api/v1/auth/me").await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let account = me["account"]["id"].as_str().expect("account id").to_owned();
    let pseud = me["active_pseud_id"].as_str().expect("pseud id").to_owned();
    (account, pseud)
}

/// Raw SQL on either backend: the repositories have no `create_category`, so
/// fixtures seed rows the way `milestone_31.rs` does.
async fn exec(harness: &Harness, sqlite: &str, postgres: &str, binds: &[&str]) {
    let db = harness.tdb.db();
    let sql = db.sql(sqlite, postgres);
    match db.backend() {
        Backend::Sqlite => {
            let mut query = sqlx::query(&sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query
                .execute(db.sqlite_pool().expect("sqlite"))
                .await
                .expect("seed sql");
        }
        Backend::Postgres => {
            let mut query = sqlx::query(&sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query
                .execute(db.postgres_pool().expect("postgres"))
                .await
                .expect("seed sql");
        }
    }
}

/// A forum category for fixtures to land in.
async fn seed_category(harness: &Harness, name: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    exec(
        harness,
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES (?, ?, 0, 0)",
        "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ($1, $2, 0, 0)",
        &[&id, name],
    )
    .await;
    id
}

/// A topic in `category`, authored by `client`.
async fn create_topic(client: &mut Client, category: &str, title: &str) -> String {
    let (status, body) = client
        .post(
            &format!("/api/v1/forums/{category}/topics"),
            json!({ "title": title }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "create topic: {body}");
    body["id"].as_str().expect("topic id").to_owned()
}

/// A post (reply) in `topic`, authored by `client`.
async fn create_post(client: &mut Client, topic: &str, body: &str) -> String {
    let (status, created) = client
        .post(&format!("/api/v1/topics/{topic}/replies"), json!({ "body": body }))
        .await;
    assert_eq!(status, StatusCode::OK, "create post: {created}");
    created["id"].as_str().expect("post id").to_owned()
}

/// A category-scoped vote type, as an operator would configure it.
async fn seed_vote_type(
    harness: &Harness,
    id: &str,
    label: &str,
    scope: &str,
    position: &str,
    cost: &str,
    negative: &str,
) {
    exec(
        harness,
        "INSERT INTO forum_vote_types (id, label, category_scope, position, weight_bp, cost, is_negative) \
         VALUES (?, ?, ?, ?, 1000, ?, ?)",
        "INSERT INTO forum_vote_types (id, label, category_scope, position, weight_bp, cost, is_negative) \
         VALUES ($1, $2, $3, $4, 1000, $5, $6)",
        &[id, label, scope, position, cost, negative],
    )
    .await;
}

/// Give an account a trust level, as the trust ladder would.
async fn grant_trust(harness: &Harness, account: &str, level: i64) {
    let basis = format!("{{\"fixture\":{level}}}");
    lorehaven_db::governance::set_trust(harness.tdb.db(), account, level, &basis)
        .await
        .expect("set trust");
}

/// Move a karma row's anchor back in time, so a 30-day month can be tested
/// without waiting one.
async fn backdate_karma(harness: &Harness, pseud: &str, days: i64) {
    let at = lorehaven_db::identity::in_seconds(-days * 86_400);
    exec(
        harness,
        "UPDATE forum_karma SET updated_at = ? WHERE pseud = ?",
        "UPDATE forum_karma SET updated_at = $1 WHERE pseud = $2",
        &[&at, pseud],
    )
    .await;
}

/// Cast a vote, asserting it was accepted.
async fn cast(client: &mut Client, post: &str, vote_type: &str) -> Value {
    let (status, body) = client
        .post(
            &format!("/api/v1/forum/posts/{post}/vote"),
            json!({ "vote_type": vote_type }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "cast {vote_type}: {body}");
    body
}


// ---------------------------------------------------------------------------
// Casting, changing, retracting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_typed_vote_is_one_per_pseud_changeable_and_retractable() {
    let harness = Harness::new("vote-basics").await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    register(&mut author, "m32-a@t.test", "m32a").await;
    let topic = create_topic(&mut author, &category, "A topic").await;
    let post = create_post(&mut author, &topic, "A post worth voting on.").await;

    let mut reader = harness.client();
    register(&mut reader, "m32-b@t.test", "m32b").await;

    let body = cast(&mut reader, &post, "insightful").await;
    assert_eq!(body["outcome"], "cast");
    assert_eq!(
        body["weight_bp"], 1000,
        "an unmoderated caster votes at full weight: {body}"
    );

    // Changing the type is a change, not a second vote.
    let body = cast(&mut reader, &post, "funny").await;
    assert_eq!(body["outcome"], "changed");

    let (status, votes) = reader
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(status, StatusCode::OK, "{votes}");
    let counts = votes["counts"].as_array().expect("counts");
    assert_eq!(counts.len(), 1, "one row per pseud: {counts:?}");
    assert_eq!(counts[0]["vote_type"], "funny");
    assert_eq!(counts[0]["count"], 1);
    assert_eq!(votes["mine"], "funny");
    assert_eq!(votes["total"], 1);

    // A second reader on the same type: the aggregate follows.
    let mut other = harness.client();
    register(&mut other, "m32-c@t.test", "m32c").await;
    cast(&mut other, &post, "funny").await;
    let (_, votes) = reader
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["counts"][0]["count"], 2, "{votes}");
    assert_eq!(votes["weighted_bp"], 2000, "{votes}");

    // A type this category does not offer is refused with a sentence.
    let (status, refused) = reader
        .post(
            &format!("/api/v1/forum/posts/{post}/vote"),
            json!({ "vote_type": "like" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["error"]["code"], "VALIDATION_FAILED");

    // Retract.
    let (status, body) = reader
        .request("DELETE", &format!("/api/v1/forum/posts/{post}/vote"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "retracted");
    assert_eq!(body["removed"], true);

    // Retracting nothing is a success that says so, not a 404 to special-case.
    let (status, body) = reader
        .request("DELETE", &format!("/api/v1/forum/posts/{post}/vote"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], false);

    let (_, votes) = reader
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["counts"][0]["count"], 1, "the reader's vote is gone");
    assert!(votes["mine"].is_null());
    assert_eq!(votes["weighted_bp"], 1000, "the remaining vote only");

    harness.cleanup().await;
}



// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_vote_budget_is_enforced_per_rolling_window_and_reported() {
    // Two votes per window: the point is the rule, not the size of the number.
    let harness = Harness::with_config("budget", |config| {
        config.forum.vote_budget = vec![(0, 2)];
    })
    .await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    register(&mut author, "m32-d@t.test", "m32d").await;
    let topic = create_topic(&mut author, &category, "Budget").await;
    let first = create_post(&mut author, &topic, "First.").await;
    let second = create_post(&mut author, &topic, "Second.").await;
    let third = create_post(&mut author, &topic, "Third.").await;

    let mut voter = harness.client();
    register(&mut voter, "m32-e@t.test", "m32e").await;

    let (status, budget) = voter.get("/api/v1/me/vote-budget").await;
    assert_eq!(status, StatusCode::OK, "{budget}");
    assert_eq!(budget["limit"], 2);
    assert_eq!(budget["spent"], 0);
    assert_eq!(budget["remaining"], 2);
    assert_eq!(budget["exhausted"], false);
    assert_eq!(budget["window_hours"], 24);
    assert!(
        budget["resets_at"].is_null(),
        "nothing is charged yet, so nothing is waiting to age out: {budget}"
    );

    let body = cast(&mut voter, &first, "insightful").await;
    assert_eq!(body["budget"]["spent"], 1, "{body}");
    let body = cast(&mut voter, &second, "insightful").await;
    assert_eq!(body["budget"]["spent"], 2, "{body}");
    assert_eq!(body["budget"]["remaining"], 0);

    // Exhaustion is reported plainly, and the vote is not recorded.
    let (status, refused) = voter
        .post(
            &format!("/api/v1/forum/posts/{third}/vote"),
            json!({ "vote_type": "insightful" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["error"]["code"], "VALIDATION_FAILED");
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("budget"),
        "the refusal names the budget: {message}"
    );

    let (_, votes) = voter
        .get(&format!("/api/v1/forum/posts/{third}/votes"))
        .await;
    assert_eq!(votes["total"], 0, "a refused vote records nothing: {votes}");

    let (_, budget) = voter.get("/api/v1/me/vote-budget").await;
    assert_eq!(budget["exhausted"], true);
    assert!(
        budget["resets_at"].is_string(),
        "the window says when it refills: {budget}"
    );

    // The budget is the account's, not the pseud's: a fresh account votes.
    let mut other = harness.client();
    register(&mut other, "m32-f@t.test", "m32f").await;
    cast(&mut other, &third, "insightful").await;

    harness.cleanup().await;
}

#[tokio::test]
async fn a_negative_vote_costs_more_budget_than_a_positive_one() {
    let harness = Harness::with_config("costs", |config| {
        config.forum.vote_budget = vec![(0, 2)];
    })
    .await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    register(&mut author, "m32-g@t.test", "m32g").await;
    let topic = create_topic(&mut author, &category, "Costs").await;
    let first = create_post(&mut author, &topic, "First.").await;
    let second = create_post(&mut author, &topic, "Second.").await;

    // The taxonomy states the costs (it is data, so it is readable).
    let mut reader = harness.client();
    register(&mut reader, "m32-h@t.test", "m32h").await;
    let (status, types) = reader
        .get(&format!("/api/v1/forum/categories/{category}/vote-types"))
        .await;
    assert_eq!(status, StatusCode::OK, "{types}");
    let items = types["items"].as_array().expect("types");
    let cost_of = |id: &str| {
        items
            .iter()
            .find(|item| item["id"] == id)
            .unwrap_or_else(|| panic!("{id} is offered: {items:?}"))["cost"]
            .as_i64()
            .expect("cost")
    };
    assert!(
        cost_of("disagree") > cost_of("insightful"),
        "a negative vote costs more: {items:?}"
    );

    // Two positive votes fit in a budget of two.
    let mut positive = harness.client();
    register(&mut positive, "m32-i@t.test", "m32i").await;
    cast(&mut positive, &first, "insightful").await;
    cast(&mut positive, &second, "insightful").await;

    // One positive vote, then a negative one, does not.
    let mut mixed = harness.client();
    register(&mut mixed, "m32-j@t.test", "m32j").await;
    cast(&mut mixed, &first, "insightful").await;
    let (status, refused) = mixed
        .post(
            &format!("/api/v1/forum/posts/{second}/vote"),
            json!({ "vote_type": "disagree" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a negative vote needs more room than is left: {refused}"
    );

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Meta-moderation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn meta_moderation_decays_a_casters_weight_not_their_voice() {
    let harness = Harness::new("meta-mod").await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    register(&mut author, "m32-k@t.test", "m32k").await;
    let topic = create_topic(&mut author, &category, "Meta-moderation").await;
    let mut posts = Vec::new();
    for index in 0..4 {
        posts.push(create_post(&mut author, &topic, &format!("Post {index}.")).await);
    }

    // The caster votes on three posts at full weight.
    let mut caster = harness.client();
    let (_, caster_pseud) = register(&mut caster, "m32-l@t.test", "m32l").await;
    for post in posts.iter().take(3) {
        let body = cast(&mut caster, post, "insightful").await;
        assert_eq!(body["weight_bp"], 1000);
    }

    // A steward, TL4. Moderators always see who voted, which is how a
    // meta-mod knows which vote to flag.
    let mut steward = harness.client();
    let (steward_account, _) = register(&mut steward, "m32-m@t.test", "m32m").await;
    grant_trust(&harness, &steward_account, 4).await;

    // Someone below TL4 may not meta-moderate at all.
    let mut regular = harness.client();
    let (regular_account, _) = register(&mut regular, "m32-n@t.test", "m32n").await;
    grant_trust(&harness, &regular_account, 3).await;
    let (status, first_votes) = steward
        .get(&format!("/api/v1/forum/posts/{}/votes", posts[0]))
        .await;
    assert_eq!(status, StatusCode::OK, "{first_votes}");
    assert_eq!(first_votes["transparency"], "individual_votes");
    let vote_id = first_votes["votes"][0]["id"]
        .as_str()
        .expect("a moderator's view carries vote ids")
        .to_owned();
    let (status, refused) = regular
        .post(
            &format!("/api/v1/forum/votes/{vote_id}/meta"),
            json!({ "fair": false }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "meta-moderation is TL4+: {refused}"
    );

    // Three unfair verdicts on the caster's three votes.
    let mut weight = 1000;
    for post in posts.iter().take(3) {
        let (_, votes) = steward
            .get(&format!("/api/v1/forum/posts/{post}/votes"))
            .await;
        let id = votes["votes"][0]["id"].as_str().expect("vote id").to_owned();
        let (status, body) = steward
            .post(
                &format!("/api/v1/forum/votes/{id}/meta"),
                json!({ "fair": false }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        weight = body["caster_weight_bp"].as_i64().expect("weight");
    }
    assert_eq!(
        weight, 100,
        "three unfair verdicts take the caster to the configured floor"
    );

    // History is not rewritten: the flagged votes still carry the weight they
    // were cast at, and so does the karma they contributed.
    let (_, flagged) = steward
        .get(&format!("/api/v1/forum/posts/{}/votes", posts[0]))
        .await;
    assert_eq!(
        flagged["weighted_bp"], 1000,
        "a verdict moves future weight, never the vote it flagged: {flagged}"
    );
    let (_, karma) = author.get("/api/v1/forum/karma").await;
    assert_eq!(
        karma["karma_bp"], 3000,
        "the karma already given stands: {karma}"
    );

    // And the caster can still vote: the next vote counts at the floor.
    let body = cast(&mut caster, &posts[3], "insightful").await;
    assert_eq!(
        body["weight_bp"], 100,
        "weight decayed, voice did not: {body}"
    );
    let (_, karma) = author.get("/api/v1/forum/karma").await;
    assert_eq!(karma["karma_bp"], 3100, "the decayed vote still counts");

    // A steward cannot meta-moderate their own vote.
    let steward_vote = cast(&mut steward, &posts[3], "funny").await;
    assert_eq!(steward_vote["outcome"], "cast");
    let (_, listed) = steward
        .get(&format!("/api/v1/forum/posts/{}/votes", posts[3]))
        .await;
    let own_id = listed["votes"]
        .as_array()
        .expect("votes")
        .iter()
        .find(|vote| vote["vote_type"] == "funny")
        .expect("the steward's own vote")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (status, refused) = steward
        .post(
            &format!("/api/v1/forum/votes/{own_id}/meta"),
            json!({ "fair": true }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a steward cannot judge their own vote: {refused}"
    );

    // A fair verdict raises the weight again: the sanction is a ratio, not a
    // mark that never washes out.
    let (_, votes) = steward
        .get(&format!("/api/v1/forum/posts/{}/votes", posts[0]))
        .await;
    let id = votes["votes"][0]["id"].as_str().expect("vote id").to_owned();
    let (status, body) = steward
        .post(
            &format!("/api/v1/forum/votes/{id}/meta"),
            json!({ "fair": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["caster_weight_bp"], 334,
        "one fair verdict in three moves the ratio, so the sanction washes out: {body}"
    );

    let _ = caster_pseud;
    harness.cleanup().await;
}


// ---------------------------------------------------------------------------
// Transparency tiers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn individual_votes_stay_hidden_until_the_author_opens_the_record() {
    let harness = Harness::new("tiers").await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    register(&mut author, "m32-o@t.test", "m32o").await;
    let topic = create_topic(&mut author, &category, "Tiers").await;
    let post = create_post(&mut author, &topic, "A post.").await;

    let mut reader = harness.client();
    let (_, reader_pseud) = register(&mut reader, "m32-p@t.test", "m32p").await;
    cast(&mut reader, &post, "insightful").await;

    let mut stranger = harness.client();
    let (stranger_account, _) = register(&mut stranger, "m32-q@t.test", "m32q").await;

    // Anonymous by default: counts are public, names are not.
    let (status, votes) = stranger
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(status, StatusCode::OK, "{votes}");
    assert_eq!(votes["transparency"], "aggregates_only");
    assert!(votes["votes"].is_null(), "no names at this tier: {votes}");
    assert_eq!(votes["total"], 1, "the aggregate is public: {votes}");
    assert_eq!(votes["author_opted_in"], false);

    // The author has not opted in either, so the author sees no names yet.
    let (_, votes) = author
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["transparency"], "aggregates_only");

    // Nobody but the author may open the record — and a stranger is told the
    // post does not exist rather than that they were refused (spec §3.3).
    let (status, _) = stranger
        .request(
            "PUT",
            &format!("/api/v1/forum/posts/{post}/vote-visibility"),
            Some(json!({ "visible": true })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = author
        .request(
            "PUT",
            &format!("/api/v1/forum/posts/{post}/vote-visibility"),
            Some(json!({ "visible": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["votes_visible"], true);

    let (_, votes) = author
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["transparency"], "individual_votes", "{votes}");
    let named = votes["votes"].as_array().expect("names");
    assert_eq!(named.len(), 1, "{votes}");
    assert_eq!(named[0]["pseud"], reader_pseud.as_str());
    assert_eq!(named[0]["vote_type"], "insightful");
    assert_eq!(votes["author_opted_in"], true);

    // Opting in opens the record to the author, not to the world.
    let (_, votes) = stranger
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["transparency"], "aggregates_only");
    assert!(votes["votes"].is_null());

    // A moderator always sees (spec §35.2).
    grant_trust(&harness, &stranger_account, 4).await;
    let (_, votes) = stranger
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["transparency"], "individual_votes", "{votes}");

    // The reader always sees their own vote, and only their own.
    let (_, mine) = reader
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(mine["mine"], "insightful");
    assert_eq!(mine["transparency"], "aggregates_only");

    // And the author can close it again.
    let (status, _) = author
        .request(
            "PUT",
            &format!("/api/v1/forum/posts/{post}/vote-visibility"),
            Some(json!({ "visible": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, votes) = author
        .get(&format!("/api/v1/forum/posts/{post}/votes"))
        .await;
    assert_eq!(votes["transparency"], "aggregates_only", "{votes}");

    harness.cleanup().await;
}

// ---------------------------------------------------------------------------
// Karma
// ---------------------------------------------------------------------------

#[tokio::test]
async fn karma_follows_weighted_votes_and_decays_on_inactivity() {
    let harness = Harness::new("karma").await;
    let category = seed_category(&harness, "General").await;
    let mut author = harness.client();
    let (_, author_pseud) = register(&mut author, "m32-r@t.test", "m32r").await;
    let topic = create_topic(&mut author, &category, "Karma").await;
    let post = create_post(&mut author, &topic, "A post that earns karma.").await;

    let mut first = harness.client();
    register(&mut first, "m32-s@t.test", "m32s").await;
    cast(&mut first, &post, "insightful").await;

    let mut second = harness.client();
    register(&mut second, "m32-t@t.test", "m32t").await;
    cast(&mut second, &post, "well_written").await;

    // Karma is received votes, at the weight they were cast with.
    let (status, karma) = author.get("/api/v1/forum/karma").await;
    assert_eq!(status, StatusCode::OK, "{karma}");
    assert_eq!(karma["karma_bp"], 2000, "{karma}");
    assert_eq!(karma["karma"], 2.0, "basis points render as a number");
    assert_eq!(karma["votes_received"], 2);
    assert_eq!(karma["weighted_received_bp"], 2000);
    assert_eq!(karma["pseud"], author_pseud.as_str());

    // The same figure is public on a profile.
    let mut reader = harness.client();
    register(&mut reader, "m32-u@t.test", "m32u").await;
    let (status, public) = reader
        .get(&format!("/api/v1/forum/karma/{author_pseud}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["karma_bp"], 2000);

    // Retracting a vote takes its contribution with it.
    let (status, _) = second
        .request("DELETE", &format!("/api/v1/forum/posts/{post}/vote"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, karma) = author.get("/api/v1/forum/karma").await;
    assert_eq!(karma["karma_bp"], 1000, "a retracted vote is not karma");

    // And karma decays 5% per 30-day month of inactivity. Backdating the
    // anchor is how a month is simulated without waiting for one; the decay
    // itself is computed from the clock on read.
    backdate_karma(&harness, &author_pseud, 60).await;
    let (_, karma) = author.get("/api/v1/forum/karma").await;
    assert_eq!(
        karma["karma_bp"], 902,
        "two inactive months at 5%: 1000 -> 950 -> 902: {karma}"
    );

    // A month that has not passed leaves it alone, and the decay does not
    // apply twice for the same month.
    let (_, again) = author.get("/api/v1/forum/karma").await;
    assert_eq!(again["karma_bp"], 902, "idempotent: {again}");

    harness.cleanup().await;
}



// ---------------------------------------------------------------------------
// Taxonomy as data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_category_taxonomy_override_changes_the_surface_without_code_changes() {
    let harness = Harness::new("taxonomy").await;
    let general = seed_category(&harness, "General").await;
    let critique = seed_category(&harness, "Critique").await;

    // The Critique category's own set, inserted as rows: no code changed.
    seed_vote_type(&harness, "constructive", "Constructive", &critique, "0", "1", "0").await;
    seed_vote_type(
        &harness, "harsh_but_fair", "Harsh but fair", &critique, "1", "1", "0",
    )
    .await;
    seed_vote_type(&harness, "needs_sources", "Needs sources", &critique, "2", "2", "1").await;

    let mut author = harness.client();
    register(&mut author, "m32-v@t.test", "m32v").await;
    let general_topic = create_topic(&mut author, &general, "General topic").await;
    let general_post = create_post(&mut author, &general_topic, "Body.").await;
    let critique_topic = create_topic(&mut author, &critique, "Critique topic").await;
    let critique_post = create_post(&mut author, &critique_topic, "Body.").await;

    let mut voter = harness.client();
    register(&mut voter, "m32-w@t.test", "m32w").await;

    // The Critique surface offers its own set, in its own order.
    let (status, types) = voter
        .get(&format!("/api/v1/forum/categories/{critique}/vote-types"))
        .await;
    assert_eq!(status, StatusCode::OK, "{types}");
    let ids: Vec<&str> = types["items"]
        .as_array()
        .expect("types")
        .iter()
        .map(|item| item["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec!["constructive", "harsh_but_fair", "needs_sources"]);

    // So it refuses the default type and accepts its own.
    let (status, refused) = voter
        .post(
            &format!("/api/v1/forum/posts/{critique_post}/vote"),
            json!({ "vote_type": "insightful" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the default type is not offered in this category: {refused}"
    );
    let body = cast(&mut voter, &critique_post, "constructive").await;
    assert_eq!(body["vote_type"], "constructive");

    // The unconfigured category still offers the default set.
    let (_, types) = voter
        .get(&format!("/api/v1/forum/categories/{general}/vote-types"))
        .await;
    let ids: Vec<&str> = types["items"]
        .as_array()
        .expect("types")
        .iter()
        .map(|item| item["id"].as_str().expect("id"))
        .collect();
    assert!(
        ids.contains(&"insightful") && ids.contains(&"disagree"),
        "the default set, unchanged: {ids:?}"
    );
    cast(&mut voter, &general_post, "insightful").await;

    harness.cleanup().await;
}


// ---------------------------------------------------------------------------
// Karma containment (spec §35.2, §0.3)
// ---------------------------------------------------------------------------

/// Collect every Rust source file under a workspace subdirectory.
fn rust_sources(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn karma_is_read_by_nothing_but_its_display_surface() {
    // Spec §35.2: karma is a display signal only — it never gates trust,
    // moderation, search ranking or credits. Two greps hold the line:
    //
    // 1. the `forum_karma` table is touched only by the typed-votes storage
    //    (the display route reads karma through its summary functions);
    // 2. no trust, ranking, credit or search path so much as mentions karma.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sources = Vec::new();
    rust_sources(&root.join("crates"), &mut sources);
    assert!(
        !sources.is_empty(),
        "the workspace walk must find the sources it polices"
    );

    let mut offenders = Vec::new();
    let mut karma_readers = Vec::new();
    for path in &sources {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if content.contains("forum_karma") {
            karma_readers.push(path.to_string_lossy().into_owned());
            if name != "typed_votes.rs" && name != "milestone_32.rs" {
                offenders.push(format!(
                    "{path:?} reads the karma table but is not the typed-votes surface"
                ));
            }
        }
        let haystack = path.to_string_lossy().to_lowercase();
        let gated_surface = ["economy", "governance", "search", "query", "ranking", "credits"]
            .iter()
            .any(|token| haystack.contains(token));
        if gated_surface && content.to_lowercase().contains("karma") {
            offenders.push(format!("{path:?} is a gated path that mentions karma"));
        }
    }

    // A walk that found nothing to protect would pass by construction.
    assert!(
        karma_readers
            .iter()
            .any(|file| file.ends_with("db/src/typed_votes.rs")),
        "the storage module must be the table's reader; found: {karma_readers:?}"
    );
    let route = root.join("crates/app/src/routes/typed_votes.rs");
    let route_body = std::fs::read_to_string(&route).expect("the display route exists");
    assert!(
        route_body.to_lowercase().contains("karma"),
        "the display route is the only surface that reads karma, so it must say so"
    );
    assert!(
        offenders.is_empty(),
        "karma leaks into a gated path:\n{}",
        offenders.join("\n")
    );
}

