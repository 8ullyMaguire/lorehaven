//! M46: Work card aggregate metrics — views, kudos, and the public metric bar.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use lorehaven_app::config::Config;
use lorehaven_app::server::{self, set_trust_proxy};
use lorehaven_app::state::AppState;
use lorehaven_db::DatabaseConfig;
use serde_json::{json, Value};
use std::path::Path;
use std::path::PathBuf;
use tower::ServiceExt;

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-metrics-{tag}-{}-{:?}",
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

struct Harness {
    _dir: PathBuf,
    tdb: test_support::TestDb,
    config: Config,
}

impl Harness {
    async fn new(tag: &str) -> Self {
        set_trust_proxy(false);
        let _ = lorehaven_app::logging::init(&lorehaven_app::config::LoggingConfig {
            filter: "error".to_owned(),
            format: lorehaven_app::config::LogFormat::Pretty,
        });
        let dir = scratch_dir(tag);
        let tdb = test_support::TestDb::connect_with_dir(tag, &dir).await;
        let config = config_for(&dir);
        Self {
            _dir: dir,
            tdb,
            config,
        }
    }

    fn client(&self) -> Client {
        Client::new(server::build_router(AppState::new(
            self.config.clone(),
            self.tdb.db().clone(),
        )))
    }
}

const PASSWORD: &str = "test-pass-1234";

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
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in &self.cookies {
            builder = builder.header(header::COOKIE, format!("{name}={value}"));
        }
        if let Some(token) = self.cookie("lorehaven_csrf").map(str::to_owned) {
            builder = builder.header("x-csrf-token", token);
        }
        let body = match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        self.capture(&response);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()));
        (status, json)
    }

    async fn post(&mut self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.request("POST", uri, Some(body)).await
    }

    async fn get(&mut self, uri: &str) -> (StatusCode, Value) {
        self.request("GET", uri, None).await
    }

    async fn register(&mut self, email: &str, handle: &str) -> String {
        let (status, body) = self
            .post(
                "/api/v1/auth/register",
                json!({
                    "email": email,
                    "password": PASSWORD,
                    "handle": handle,
                    "display_name": handle,
                    "age_band": "adult",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "register {handle}: {body}");
        let (status, me) = self.get("/api/v1/auth/me").await;
        assert_eq!(status, StatusCode::OK, "{me}");
        me["account"]["id"].as_str().expect("account id").to_owned()
    }

    async fn create_work(&mut self, title: &str) -> String {
        let (status, body) = self.post("/api/v1/works", json!({ "title": title })).await;
        assert_eq!(status, StatusCode::CREATED, "create work {title}: {body}");
        body["id"].as_str().expect("work id").to_owned()
    }

    async fn add_chapter(&mut self, work_id: &str) {
        let (status, body) = self
            .post(
                &format!("/api/v1/works/{work_id}/chapters"),
                json!({ "title": "One" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "add chapter: {body}");
        let chapter = body["id"].as_str().expect("chapter id").to_owned();
        let chapter_version = body["version"].as_i64().expect("chapter version");
        let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A chapter with enough words to have a middle." }] }] });
        let (status, body) = self
            .request(
                "PATCH",
                &format!("/api/v1/chapters/{chapter}"),
                Some(json!({ "expected_version": chapter_version, "document": doc })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "save chapter: {body}");
    }

    async fn publish_work(&mut self, work_id: &str) {
        // Fetch the current version first, then publish with it.
        let (status, body) = self.get(&format!("/api/v1/works/{work_id}")).await;
        assert_eq!(status, StatusCode::OK, "fetch for version: {body}");
        let version = body["version"].as_i64().expect("work version");
        let (status, body) = self
            .post(
                &format!("/api/v1/works/{work_id}/publish"),
                json!({ "expected_version": version, "idempotency_key": format!("pub-{work_id}") }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "publish work: {body}");
    }
}

#[tokio::test]
async fn public_work_excludes_metrics_when_owner_opted_out() {
    let h = Harness::new("opted-out").await;
    let mut owner = h.client();
    owner.register("owner-o@example.test", "opted-out").await;
    let work_id = owner.create_work("Opted Out").await;
    owner.add_chapter(&work_id).await;
    owner.publish_work(&work_id).await;

    // Opt out of public ratings.
    let (status, body) = owner.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let version = body["version"].as_i64().expect("version");
    let (status, _) = owner
        .request(
            "PATCH",
            &format!("/api/v1/works/{work_id}"),
            Some(json!({ "show_public_ratings": false, "expected_version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Anonymous reader: no metrics field, or null.
    let mut reader = h.client();
    let (_, body) = reader.get(&format!("/api/v1/works/{work_id}")).await;
    assert!(
        body["metrics"].is_null(),
        "opted-out work must hide metrics: {body}"
    );
}

#[tokio::test]
async fn public_work_shows_metrics_when_allowed() {
    let h = Harness::new("opted-in").await;
    let mut owner = h.client();
    owner.register("owner-i@example.test", "opted-in").await;
    let work_id = owner.create_work("Opted In").await;
    owner.add_chapter(&work_id).await;
    owner.publish_work(&work_id).await;

    // Read as anonymous: metrics present (all zero).
    let mut reader = h.client();
    let (_, body) = reader.get(&format!("/api/v1/works/{work_id}")).await;
    assert!(
        !body["metrics"].is_null(),
        "opted-in work must show metrics: {body}"
    );
    assert_eq!(body["metrics"]["views"], 0);

    // Kudos as another reader.
    let mut kudoser = h.client();
    kudoser.register("kudoser@example.test", "kudoser").await;
    let (status, kudo_body) = kudoser
        .post(&format!("/api/v1/works/{work_id}/kudos"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "kudos: {kudo_body}");
    assert_eq!(kudo_body["kudoed"], true);

    // The kudos count is now 1.
    let (_, body) = reader.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(body["metrics"]["kudos"], 1);

    // Second kudos from same account: removes (idempotent toggle).
    let (status, kudo_body) = kudoser
        .post(&format!("/api/v1/works/{work_id}/kudos"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kudo_body["kudoed"], false);

    let (_, body) = reader.get(&format!("/api/v1/works/{work_id}")).await;
    assert_eq!(body["metrics"]["kudos"], 0, "kudos must toggle off");
}

#[tokio::test]
async fn kudos_requires_authentication() {
    let h = Harness::new("auth-required").await;
    let mut owner = h.client();
    owner.register("owner-a@example.test", "auth-req").await;
    let work_id = owner.create_work("Auth Required").await;
    owner.add_chapter(&work_id).await;
    owner.publish_work(&work_id).await;

    let mut anon = h.client();
    let (status, _) = anon
        .post(&format!("/api/v1/works/{work_id}/kudos"), json!({}))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "kudos must require auth");
}
