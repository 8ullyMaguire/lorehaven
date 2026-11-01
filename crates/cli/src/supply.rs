//! §58's supply surface at the terminal (M45-25).
//!
//! ```text
//! dh supply keystones --limit 10      who holds this catalogue up
//! dh supply droughts                 their conversation position, or why it is unmeasurable
//! dh supply recompute                run the detection pass (operator)
//! dh supply route [--author KEY]     ask for one work to read and comment on
//! dh supply routing                  what I have been given
//! dh supply complete <ID>            done — and the patronage pays
//! ```
//!
//! # The same design rule as `dh rec`
//!
//! Every threshold, every percentile and every refusal is the server's. This
//! module sends what was typed and prints what came back, which is why `dh supply
//! droughts` on an instance with no comments prints the server's reason rather
//! than an empty table: an empty table reads as "no droughts", and the truth is
//! "droughts cannot be measured yet".

use clap::Subcommand;
use serde_json::Value;

use crate::rec::{environment, failure_for, Failure};
use crate::Cli;

#[derive(Debug, Clone, Subcommand)]
pub enum SupplyArgs {
    /// The keystone authors: who carries the catalogue (operator).
    Keystones {
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// Their conversation position — or the reason it cannot be measured (operator).
    Droughts {
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// Recompute the keystone snapshot (operator).
    Recompute {
        /// An author needs at least this many engaged works.
        #[arg(long, default_value_t = 2)]
        min_works: i64,
        /// …and at least this mean within-source percentile, as a fraction.
        #[arg(long, default_value_t = 0.75)]
        min_pct: f64,
    },
    /// Ask for one work to read and comment on.
    Route {
        /// Pin the author by key, instead of taking the highest-scoring keystone.
        #[arg(long, value_name = "AUTHOR")]
        author: Option<String>,
    },
    /// What I have been given, newest first.
    Routing {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Mark a routing complete; the patronage pays once.
    Complete {
        /// The routing id printed by `dh supply route`.
        id: String,
    },
}

pub async fn run(cli: &Cli, args: &SupplyArgs) -> Result<(), Failure> {
    // Every door needs a session: the operator doors check a role the server
    // holds, and the reader doors are keyed by account. Refused here so the
    // message says what to set rather than quoting a 401.
    let token = cli.token.as_deref().ok_or_else(|| {
        environment("every supply door needs a session: set DARKHOLD_TOKEN")
    })?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| environment(format!("http client: {e}")))?;

    let (method, url, body) = match args {
        SupplyArgs::Keystones { limit } => {
            check_limit(*limit)?;
            (
                reqwest::Method::GET,
                format!("{}/api/v1/supply/keystones?limit={limit}", cli.base_url),
                None,
            )
        }
        SupplyArgs::Droughts { limit } => {
            check_limit(*limit)?;
            (
                reqwest::Method::GET,
                format!("{}/api/v1/supply/droughts?limit={limit}", cli.base_url),
                None,
            )
        }
        SupplyArgs::Recompute { min_works, min_pct } => {
            if !(1..=25).contains(min_works) {
                return Err(environment(
                    "min_works must be between 1 and 25; a keystone is a handful of authors",
                ));
            }
            if !(0.0..=1.0).contains(min_pct) {
                return Err(environment(
                    "min_pct is a fraction of a source's engaged rows, so it lies between 0 and 1",
                ));
            }
            (
                reqwest::Method::POST,
                format!("{}/api/v1/supply/recompute", cli.base_url),
                Some(serde_json::json!({ "min_works": min_works, "min_pct": min_pct })),
            )
        }
        SupplyArgs::Route { author } => (
            reqwest::Method::POST,
            format!("{}/api/v1/supply/route", cli.base_url),
            Some(match author {
                Some(a) => serde_json::json!({ "author": a }),
                None => serde_json::json!({}),
            }),
        ),
        SupplyArgs::Routing { limit } => {
            check_limit(*limit)?;
            (
                reqwest::Method::GET,
                format!("{}/api/v1/supply/routing?limit={limit}", cli.base_url),
                None,
            )
        }
        SupplyArgs::Complete { id } => (
            reqwest::Method::POST,
            format!("{}/api/v1/supply/routing/{id}/complete", cli.base_url),
            None,
        ),
    };

    let mut request = client.request(method, &url).bearer_auth(token);
    if let Some(b) = body {
        request = request.json(&b);
    }
    let response = request.send().await.map_err(|e| match e.is_connect() {
        true => environment(format!(
            "nothing listening at {} — is the server running? (--base-url)",
            cli.base_url
        )),
        false => environment(format!("request failed: {e}")),
    })?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| environment(format!("reading the response: {e}")))?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        return Err(failure_for(status, &body, &cli.base_url));
    }

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        );
        return Ok(());
    }

    print!("{}", render(args, &body));
    Ok(())
}

fn check_limit(limit: i64) -> Result<(), Failure> {
    if (1..=100).contains(&limit) {
        Ok(())
    } else {
        Err(environment("limit must be between 1 and 100"))
    }
}

/// What a person reads, per subcommand.
fn render(args: &SupplyArgs, body: &Value) -> String {
    match args {
        SupplyArgs::Keystones { .. } => {
            let rows = body["keystones"].as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                return format!(
                    "no keystone snapshot yet\n\n{}\n",
                    body["reason"].as_str().unwrap_or("")
                );
            }
            let mut out = String::new();
            out.push_str(&format!(
                "{:<7} {:>3} {:>8} {:>7} {:>7}  {}\n",
                "score", "n", "mean", "taste", "score", "author"
            ));
            for r in rows {
                let pct = |k: &str| r[k].as_i64().unwrap_or(0) as f64 / 10_000.0;
                out.push_str(&format!(
                    "{:<7.3} {:>3} {:>8.3} {:>7.3} {:>7.3}  {}\n",
                    pct("score_bp"),
                    r["engaged_works"].as_i64().unwrap_or(0),
                    pct("mean_pct_bp"),
                    pct("taste_bonus_bp"),
                    pct("score_bp"),
                    r["display_name"].as_str().unwrap_or(""),
                ));
            }
            out.push_str(&format!(
                "\n{} keystones, computed {}\n",
                body["keystones"].as_array().map(|a| a.len()).unwrap_or(0),
                rows_first_computed_at(body).unwrap_or_default()
            ));
            out
        }
        SupplyArgs::Droughts { .. } => {
            let measured = body["measured"].as_bool().unwrap_or(false);
            if !measured {
                return format!(
                    "not measured: {}\n\ncomments: {}, ratings: {}\n",
                    body["reason"].as_str().unwrap_or(""),
                    body["comments_total"].as_i64().unwrap_or(0),
                    body["ratings_total"].as_i64().unwrap_or(0),
                );
            }
            let rows = body["authors"].as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                return format!(
                    "no keystone authors to check ({} comments, {} ratings)\n",
                    body["comments_total"].as_i64().unwrap_or(0),
                    body["ratings_total"].as_i64().unwrap_or(0)
                );
            }
            let mut out = format!(
                "{:<40} {:>5} {:>10} {:>8}\n",
                "author", "works", "with talk", "comments"
            );
            for r in rows {
                out.push_str(&format!(
                    "{:<40} {:>5} {:>10} {:>8}\n",
                    r["display_name"].as_str().unwrap_or(""),
                    r["engaged_works"].as_i64().unwrap_or(0),
                    r["works_with_conversation"].as_i64().unwrap_or(0),
                    r["comments"].as_i64().unwrap_or(0),
                ));
            }
            out
        }
        SupplyArgs::Recompute { .. } => format!(
            "{} engaged rows, {} authors, {} keystones \
             (min_works {}, mean percentile >= {:.2}); taste weights read: {}\n",
            body["engaged_rows"].as_i64().unwrap_or(0),
            body["authors_seen"].as_i64().unwrap_or(0),
            body["keystones"].as_i64().unwrap_or(0),
            body["min_works"].as_i64().unwrap_or(0),
            body["min_pct_bp"].as_i64().unwrap_or(0) as f64 / 10_000.0,
            body["taste_profile_rows"].as_i64().unwrap_or(0),
        ),
        SupplyArgs::Route { .. } => match body["route"].as_object() {
            Some(r) => format!(
                "{}\n  {}\n  {} · {}\n  id {}\n\n{}\n",
                r["title"].as_str().unwrap_or(""),
                r["source_key"].as_str().unwrap_or(""),
                r["author_key"].as_str().unwrap_or(""),
                "",
                r["id"].as_str().unwrap_or(""),
                r["reason"].as_str().unwrap_or(""),
            ),
            None => format!("{}\n", body["reason"].as_str().unwrap_or("")),
        },
        SupplyArgs::Routing { .. } => {
            let rows = body["routing"].as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                return "nothing routed yet — `dh supply route` asks for one\n".to_string();
            }
            let mut out = String::new();
            for r in rows {
                out.push_str(&format!(
                    "{}  {}{}\n    {} ({})\n",
                    r["created_at"].as_str().unwrap_or(""),
                    r["title"].as_str().unwrap_or(""),
                    match r["completed_at"].as_str() {
                        Some(_) => format!(
                            "  [done, {} bp]",
                            r["credit_bp"].as_i64().unwrap_or(0)
                        ),
                        None => "  [open]".to_string(),
                    },
                    r["author_key"].as_str().unwrap_or(""),
                    r["id"].as_str().unwrap_or(""),
                ));
            }
            out
        }
        SupplyArgs::Complete { .. } => format!(
            "done: {} — paid {} bp\n",
            body["work_id"].as_str().unwrap_or(""),
            body["credit_bp"].as_i64().unwrap_or(0)
        ),
    }
}

fn rows_first_computed_at(body: &Value) -> Option<String> {
    body["keystones"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r["computed_at"].as_str())
        .map(str::to_string)
}
