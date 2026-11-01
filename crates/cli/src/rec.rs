//! `dh rec` — the anchor verb.
//!
//! Two server routes, chosen by whether an engine was asked for:
//!
//! | invocation | route | why |
//! |---|---|---|
//! | `dh rec --fandom naruto --updated 7d` | `/api/v1/catalogue` | public, metric order, no token |
//! | `dh rec … --engine rrf` | `/api/v1/rec/run` | the engines read your history, so they need a session |
//!
//! The split is deliberate rather than convenient: the catalogue is a public
//! surface and a user who only wants "top Naruto fics by likes" should not need a
//! token to ask. The moment an engine is named, the request is about *this*
//! account's recommendations, and the server refuses it without one.

use anyhow::Result;
use clap::Args;
use serde_json::Value;

use crate::Cli;

/// Options for `rec`.
#[derive(Debug, Clone, Args)]
pub struct RecArgs {
    /// Restrict to one fandom, spelled as the catalogue spells it.
    #[arg(long, value_name = "NAME")]
    pub fandom: Option<String>,

    /// Window on when the author last touched it: `7d`, `>30d`, `@2026-10-27`,
    /// `2026-01-01..2026-10-15`.
    #[arg(long, value_name = "WINDOW")]
    pub updated: Option<String>,

    /// Window on when the crawler last rechecked it.
    #[arg(long, value_name = "WINDOW")]
    pub refreshed: Option<String>,

    /// Window on when it entered this catalogue.
    #[arg(long, value_name = "WINDOW")]
    pub added: Option<String>,

    /// Restrict to one source: `sb`, `sv`, `qq`, `ao3`, `royalroad`, …
    #[arg(long, value_name = "KEY")]
    pub source: Option<String>,

    /// Only rows with at least this many likes.
    #[arg(long, value_name = "N")]
    pub min_likes: Option<i64>,

    /// Substring over title and author.
    #[arg(long, value_name = "TEXT")]
    pub q: Option<String>,

    /// The metric that orders the page. `--list-metrics` prints every name with
    /// whether it can be computed today.
    #[arg(long, value_name = "METRIC")]
    pub sort: Option<String>,

    /// Print the metric vocabulary and exit. No server is contacted.
    #[arg(long)]
    pub list_metrics: bool,

    /// Order the candidates with an engine: `catalogue`, `legacy`, `rrf`, or a
    /// registered strategy name. Needs `--token`.
    #[arg(long, value_name = "ENGINE")]
    pub engine: Option<String>,

    /// Print what each engine contributed. Implies `--engine rrf` when no engine
    /// was named, because there is nothing to explain without one.
    #[arg(long)]
    pub explain: bool,

    /// How many rows to print.
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub limit: i64,

    /// Similar to this work: its id, its source URL, or its title.
    #[arg(long, value_name = "WORK")]
    pub seed: Option<String>,

    /// Restrict to one author's pseud: exact, case-insensitive.
    #[arg(long, value_name = "PSEUD")]
    pub author: Option<String>,

    /// Drop what you are already reading. Reads your history, so it needs a token.
    #[arg(long)]
    pub exclude_read: bool,
}

/// What a failed run must tell the shell.
///
/// The exit codes are the contract a script depends on: `2` means the request
/// was wrong (bad fandom, unknown metric, missing token) and retrying it
/// unchanged cannot help; `1` means something environmental went wrong and it
/// might.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub code: u8,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn bad_request(message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        code: 2,
    }
}

/// Refuse a `--sort` word that is not in the vocabulary, before any request.
fn validate_sort(raw: &str) -> Result<(), Failure> {
    match darkhold_domain::quality::Metric::parse(raw) {
        Some(_) => Ok(()),
        None => Err(bad_request(format!(
            "--sort {raw}: unknown metric. `dh rec --list-metrics` prints the vocabulary"
        ))),
    }
}

pub(crate) fn environment(message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        code: 1,
    }
}

/// Which route this invocation talks to.
///
/// `explain` implies an engine: there is nothing to explain about a plain metric
/// ordering, and answering "no strategies ran" to a `--explain` would be a worse
/// experience than running the default engine and showing it.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Catalogue,
    Engine(String),
}

impl Target {
    pub fn resolve(args: &RecArgs, configured: &str) -> Self {
        // A seed is an order of its own, and it is answered by the catalogue
        // route: `--seed X --explain` explains the neighbours (the shared
        // tokens), not an engine's contribution.
        if args.seed.is_some() {
            return Self::Catalogue;
        }
        match (args.engine.as_deref(), args.explain) {
            (Some(engine), _) => Self::Engine(engine.to_owned()),
            (None, true) => Self::Engine(configured.to_owned()),
            (None, false) => Self::Catalogue,
        }
    }
}

/// Percent-encode a query value.
///
/// Written out rather than taking a dependency: the five values this program
/// sends are a name, a window, a metric, a source key and free text, and an
/// encoder that mishandles `&` in a title search is the only thing that could go
/// wrong with it.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Build the request URL: the route, then every parameter that was given.
pub fn url(base_url: &str, target: &Target, args: &RecArgs) -> String {
    let path = match target {
        Target::Catalogue => "/api/v1/catalogue",
        Target::Engine(_) => "/api/v1/rec/run",
    };
    let mut params: Vec<(String, String)> = Vec::new();
    let mut push = |key: &str, value: Option<&str>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            params.push((key.to_owned(), value.to_owned()));
        }
    };
    push("fandom", args.fandom.as_deref());
    push("updated", args.updated.as_deref());
    push("refreshed", args.refreshed.as_deref());
    push("added", args.added.as_deref());
    push("source", args.source.as_deref());
    push("q", args.q.as_deref());
    push("sort", args.sort.as_deref());
    push("seed", args.seed.as_deref());
    push("author", args.author.as_deref());
    if args.exclude_read {
        params.push(("exclude_read".to_owned(), "1".to_owned()));
    }
    if let Some(min_likes) = args.min_likes {
        params.push(("min_likes".to_owned(), min_likes.to_string()));
    }
    params.push(("limit".to_owned(), args.limit.to_string()));
    if let Target::Engine(engine) = target {
        params.push(("engine".to_owned(), engine.clone()));
        if args.explain {
            params.push(("explain".to_owned(), "1".to_owned()));
        }
    }
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}{path}?{query}", base_url.trim_end_matches('/'))
}

/// A catalogue row, as this program prints it.
///
/// Only the keys listed here are read, and that is the point: the server can add
/// a field to its page without moving a column under a script's nose.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub title: String,
    pub author: String,
    pub source: String,
    pub likes: i64,
    pub words: Option<i64>,
    pub status: String,
    pub updated: String,
    /// A quality metric's score, when the page carries one.
    pub score: Option<f64>,
    /// False when the metric could not measure the row (`measured: false`).
    pub measured: bool,
    /// The tokens that carried a seed search's similarity, when the page is a
    /// seed page. Empty for every other kind of page.
    pub shared: Vec<String>,
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn number(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

/// Read the rows out of either route's page shape.
///
/// Both routes put the catalogue row's fields at the top level of each item —
/// `/rec/run` flattens the row and adds `rec_score`/`rec_rank` beside it — so one
/// reader serves both, and a change that un-flattened one of them would fail the
/// render tests rather than silently printing blanks.
pub fn rows(page: &Value) -> Vec<Row> {
    page.get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| Row {
                    title: text(item, "title"),
                    author: text(item, "author_text"),
                    source: text(item, "source_key"),
                    likes: number(item, "likes").unwrap_or(0),
                    words: number(item, "word_count"),
                    status: text(item, "status"),
                    updated: {
                        // The stored shape is `YYYY-MM-DDTHH:MM:SSZ`; the date is
                        // the part a reader scans. A metric page carries the
                        // source date as `updated_at`, because the catalogue row
                        // itself does not hold it. A row with no date prints as
                        // empty rather than as "1970".
                        let raw = match text(item, "source_updated_at") {
                            empty if empty.is_empty() => text(item, "updated_at"),
                            found => found,
                        };
                        raw.split('T').next().unwrap_or("").to_owned()
                    },
                    score: item.get("score").and_then(Value::as_f64),
                    measured: item
                        .get("measured")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    shared: item
                        .get("shared")
                        .and_then(Value::as_array)
                        .map(|tokens| {
                            tokens
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `21.0k`, `1.2M`, or the plain number.
fn compact(value: i64) -> String {
    match value {
        v if v >= 1_000_000 => format!("{:.1}M", v as f64 / 1_000_000.0),
        v if v >= 1_000 => format!("{:.1}k", v as f64 / 1_000.0),
        v => v.to_string(),
    }
}

/// Pad to `width` columns, counting characters rather than bytes.
fn pad(value: &str, width: usize) -> String {
    let len = value.chars().count();
    match len >= width {
        // Truncation is explicit: a title that silently overflows pushes every
        // column right and makes the table unreadable in a narrow terminal.
        true => format!("{}…", value.chars().take(width.saturating_sub(1)).collect::<String>()),
        false => format!("{value}{}", " ".repeat(width - len)),
    }
}

/// The table.
///
/// The score column appears only on a metric-ranked page: a column of `—` would
/// suggest every row was measured, which is the one thing a metric page must not
/// imply. Unmeasured rows are marked with `~` for the same reason.
pub fn table(page: &Value) -> String {
    let rows = rows(page);
    let scored = rows.iter().any(|row| row.score.is_some());
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        let marker = if row.measured { ' ' } else { '~' };
        if scored {
            out.push_str(&format!(
                "{:>3}{} {:>6}  {:>6} likes  {:>9} words  {} {}  {} by {} [{}]\n",
                index + 1,
                marker,
                row.score
                    .map(|score| format!("{score:.3}"))
                    .unwrap_or_else(|| "—".to_owned()),
                compact(row.likes),
                row.words.map(compact).unwrap_or_else(|| "—".to_owned()),
                pad(&row.status, 8),
                row.updated,
                pad(&row.title, 44),
                pad(&row.author, 18),
                row.source,
            ));
            if !row.shared.is_empty() {
                out.push_str(&format!("     shared: {}\n", row.shared.join(", ")));
            }
        } else {
            out.push_str(&format!(
                "{:>3}  {:>6} likes  {:>9} words  {} {}  {} by {} [{}]\n",
                index + 1,
                compact(row.likes),
                row.words.map(compact).unwrap_or_else(|| "—".to_owned()),
                pad(&row.status, 8),
                row.updated,
                pad(&row.title, 44),
                pad(&row.author, 18),
                row.source,
            ));
        }
    }
    out
}

/// The gate line for a metric page: how many rows the metric could actually
/// score, and how many matched.
///
/// Without it a percentile page reads as a verdict on the whole catalogue, when
/// on the live catalogue it is a verdict on 0.9 % of it.
pub fn gate(page: &Value) -> Option<String> {
    let ranked = page.get("ranked").and_then(Value::as_i64)?;
    let total = page.get("total").and_then(Value::as_i64)?;
    let metric = page.get("metric").and_then(Value::as_str).unwrap_or("metric");
    // A seed page has three numbers, not two, and the third is the one that stops
    // it reading as a claim about the catalogue: `scanned` is what the ranker saw.
    if metric == "similarity" {
        let scanned = page.get("scanned").and_then(Value::as_i64).unwrap_or(ranked);
        let seed = page
            .get("seed")
            .and_then(|seed| seed.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        return Some(format!(
            "similar to \"{seed}\": {ranked} neighbours among the {scanned} rows scanned, \
             of {total} matching the filters"
        ));
    }
    let share = match total {
        0 => 0.0,
        total => ranked as f64 * 100.0 / total as f64,
    };
    let mut line = format!(
        "{metric}: ranked {ranked} of {total} matching rows ({share:.1}% carry an endorsement count)"
    );
    if ranked < total {
        line.push_str("; the rest are ordered by metadata and marked ~");
    }
    Some(line)
}

/// The metric vocabulary, as the CLI prints it.
pub fn metrics_list() -> String {
    use darkhold_domain::quality::{Metric, Support};
    let mut out = String::from("QUALITY METRICS — dh rec --sort <name>\n\n");
    for metric in Metric::ALL {
        let (status, detail) = match metric.support() {
            Support::Ranked => ("ready", metric.help().to_owned()),
            Support::Triage => ("triage", metric.help().to_owned()),
            Support::Raw => ("raw", metric.help().to_owned()),
            Support::Blocked(reason) => ("blocked", format!("{} — {reason}", metric.help())),
        };
        let name = match metric.support() {
            Support::Blocked(_) => format!("{}*", metric.as_str()),
            _ => metric.as_str().to_owned(),
        };
        out.push_str(&format!("  {} {} {}\n", pad(&name, 20), pad(status, 8), detail));
    }
    out.push_str("\nraw = a stored column ordered in SQL. triage = metadata order for rows with no endorsement data.\n");
    out.push_str("* = refused with the reason and the work item that unblocks it.\n");
    out
}

/// The coverage line, when the page carries one.
///
/// Printed whenever a fandom was filtered on, because the honest answer to
/// "Naruto fics" is "265 tagged ones": 94.6 % of the live catalogue has no fandom
/// metadata, and a reader who is not told reads a short page as a bad ranking.
pub fn coverage(page: &Value) -> Option<String> {
    let tagged = page.get("fandom_tagged_rows").and_then(Value::as_i64)?;
    let total = page.get("catalogue_rows").and_then(Value::as_i64)?;
    let fandom = page.get("fandom").and_then(Value::as_str)?;
    let share = match total {
        0 => 0.0,
        total => tagged as f64 * 100.0 / total as f64,
    };
    let rankable = page
        .get("fandom_tagged_with_likes")
        .and_then(Value::as_i64)
        .unwrap_or(tagged);
    let ranking = match rankable {
        0 => " — and none of them publishes a like count, so this order is ties only".to_owned(),
        n if n < tagged => format!(", {n} of them with a like count"),
        _ => String::new(),
    };
    Some(format!(
        "{fandom}: {tagged} of {total} catalogue rows carry a fandom tag ({share:.1}%){ranking}"
    ))
}

/// The explanation block, when strategies were reported.
pub fn explanation(page: &Value) -> Option<String> {
    let strategies = page.get("strategies").and_then(Value::as_array)?;
    if strategies.is_empty() {
        return None;
    }
    let engine = page.get("engine").and_then(Value::as_str).unwrap_or("?");
    let candidates = page.get("candidates").and_then(Value::as_i64).unwrap_or(0);
    let mut out = format!("engine {engine} over {candidates} candidates\n\n");
    out.push_str(&format!(
        "{}  {:>8}  {:>7}  head\n",
        pad("strategy", 18),
        "produced",
        "matched"
    ));
    for strategy in strategies {
        let name = strategy.get("name").and_then(Value::as_str).unwrap_or("?");
        let produced = strategy.get("produced").and_then(Value::as_u64).unwrap_or(0);
        let matched = strategy.get("matched").and_then(Value::as_u64).unwrap_or(0);
        let head = strategy
            .get("top")
            .and_then(Value::as_array)
            .map(|top| {
                top.iter()
                    .filter_map(|pair| pair.get("score").and_then(Value::as_f64))
                    .map(|score| format!("{score:.4}"))
                    .collect::<Vec<_>>()
                    .join("  ")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "{}  {:>8}  {:>7}  {head}\n",
            pad(name, 18),
            produced,
            matched
        ));
    }
    Some(out)
}

/// Turn the server's refusal into something a person can act on.
pub fn failure_for(status: u16, body: &Value, base_url: &str) -> Failure {
    let message = body
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| body.to_string());
    match status {
        401 | 403 => bad_request(format!(
            "{message}\nthis engine reads your own history, so it needs a session: \
             set DARKHOLD_TOKEN (or pass --token)"
        )),
        400 | 404 | 422 => bad_request(message),
        500..=599 => environment(format!("{status} from {base_url}: {message}")),
        other => environment(format!("{other} from {base_url}: {message}")),
    }
}

/// Run `dh rec`.
pub async fn run(cli: &Cli, args: &RecArgs) -> Result<(), Failure> {
    if args.list_metrics {
        print!("{}", metrics_list());
        return Ok(());
    }
    // Refuse an unknown metric here, where the message can point at the
    // vocabulary, rather than letting the server answer `unknown sort` after a
    // round trip.
    if let Some(raw) = args.sort.as_deref() {
        validate_sort(raw)?;
    }
    if !(1..=100).contains(&args.limit) {
        return Err(bad_request("limit must be between 1 and 100"));
    }
    // These come before the engine check on purpose: `--seed X --engine rrf` is a
    // contradiction whatever the token situation is, and answering it with
    // "set DARKHOLD_TOKEN" sends the user to fix the wrong thing.
    if args.seed.is_some() && args.sort.is_some() {
        return Err(bad_request(
            "--seed already decides the order; --sort would be silently ignored",
        ));
    }
    if args.seed.is_some() && args.engine.is_some() {
        return Err(bad_request(
            "--seed orders by similarity and --engine orders by your history; pick one",
        ));
    }
    // An engine request without a token is refused here rather than by the
    // server, so the message can say what to do instead of quoting a 401.
    let engine = args.engine.clone();
    if engine.is_some() && cli.token.is_none() {
        return Err(bad_request(format!(
            "--engine {} reads your own reading history, so it needs a session: \
             set DARKHOLD_TOKEN",
            engine.unwrap_or_default()
        )));
    }
    if args.exclude_read && cli.token.is_none() {
        return Err(bad_request(
            "--exclude-read needs your reading history, so it needs a session: set DARKHOLD_TOKEN",
        ));
    }
    // `explain` without an engine means "the configured engine", and only the
    // server knows which that is; ask for rrf, which is what pluggable mode
    // serves, and let the server's refusal say so if it is disabled.
    let target = Target::resolve(args, "rrf");
    let url = url(&cli.base_url, &target, args);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| environment(format!("http client: {e}")))?;
    let mut request = client.get(&url);
    if let Some(token) = &cli.token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| {
        // The most common failure by far is "nothing is listening", and the
        // generic reqwest text for it names neither the address nor the fix.
        match e.is_connect() {
            true => environment(format!(
                "nothing listening at {} — is the server running? (--base-url)",
                cli.base_url
            )),
            false => environment(format!("request failed: {e}")),
        }
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

    let printed = table(&body);
    if printed.is_empty() {
        eprintln!("no rows matched");
    } else {
        print!("{printed}");
    }
    if let Some(line) = gate(&body) {
        println!("\n{line}");
    }
    if let Some(line) = coverage(&body) {
        println!("\n{line}");
    }
    if let Some(block) = explanation(&body) {
        println!("\n{block}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(argv: &[&str]) -> (Cli, RecArgs) {
        let cli = Cli::try_parse_from(argv).expect("parses");
        let crate::Command::Rec(rec) = cli.command.clone() else {
            panic!("'rec' is the subcommand this test builds")
        };
        (cli, *rec)
    }

    #[test]
    fn rec_parses_the_anchor_ask() {
        let (cli, rec) = args(&[
            "dh", "rec", "--fandom", "naruto", "--updated", "7d", "--engine", "rrf", "--sort",
            "wilson", "--explain",
        ]);
        assert_eq!(rec.fandom.as_deref(), Some("naruto"));
        assert_eq!(rec.updated.as_deref(), Some("7d"));
        assert_eq!(rec.engine.as_deref(), Some("rrf"));
        // Carried verbatim, not validated here: the metric vocabulary is the
        // server's, and a client-side alias table would be a second definition
        // of it. `wilson` is M63-03's, and until then the server answers
        // `unknown sort: wilson` — which is the correct refusal, in one place.
        assert_eq!(rec.sort.as_deref(), Some("wilson"));
        assert!(rec.explain);
        assert_eq!(cli.base_url, "http://127.0.0.1:8081");
        assert_eq!(rec.limit, 20);
    }

    #[test]
    fn explain_without_an_engine_picks_one_rather_than_explaining_nothing() {
        let (_, rec) = args(&["dh", "rec", "--fandom", "naruto", "--explain"]);
        assert_eq!(
            Target::resolve(&rec, "rrf"),
            Target::Engine("rrf".to_owned())
        );
        let (_, plain) = args(&["dh", "rec", "--fandom", "naruto"]);
        assert_eq!(Target::resolve(&plain, "rrf"), Target::Catalogue);
    }

    #[test]
    fn the_url_carries_only_what_was_asked_for() {
        let (cli, rec) = args(&["dh", "rec", "--fandom", "Harry Potter", "--updated", ">30d"]);
        let request_url = url(&cli.base_url, &Target::Catalogue, &rec);
        assert_eq!(
            request_url,
            "http://127.0.0.1:8081/api/v1/catalogue?fandom=Harry%20Potter&updated=%3E30d&limit=20",
            "a space and a `>` have to be encoded or the window grammar breaks"
        );

        let (cli, rec) = args(&[
            "dh", "rec", "--engine", "rrf", "--fandom", "naruto", "--explain", "--limit", "5",
        ]);
        let request_url = url(&cli.base_url, &Target::Engine("rrf".to_owned()), &rec);
        assert!(request_url.starts_with("http://127.0.0.1:8081/api/v1/rec/run?"));
        assert!(request_url.contains("engine=rrf"));
        assert!(request_url.contains("explain=1"));
        assert!(request_url.contains("limit=5"));
    }

    fn page(items: Vec<Value>) -> Value {
        serde_json::json!({ "items": items, "sort": "likes", "limit": 20, "offset": 0 })
    }

    fn item(title: &str, likes: i64) -> Value {
        serde_json::json!({
            "id": "id", "source_key": "sb", "source_work_key": "w", "title": title,
            "author_text": "Esama", "summary": "", "word_count": 21044, "status": "complete",
            "likes": likes, "views": null, "source_url": "https://x.invalid",
            "source_updated_at": "2026-10-27T14:02:11Z",
        })
    }

    #[test]
    fn the_table_prints_the_columns_a_reader_scans() {
        let rendered = table(&page(vec![item("Naruto: A Quiet Sort", 812)]));
        assert!(rendered.contains("812 likes"), "{rendered}");
        assert!(rendered.contains("21.0k words"), "{rendered}");
        assert!(rendered.contains("2026-10-27"), "the date, not the instant");
        assert!(rendered.contains("Naruto: A Quiet Sort"), "{rendered}");
        assert!(rendered.contains("by Esama"), "{rendered}");
        assert!(rendered.contains("[sb]"), "{rendered}");
    }

    #[test]
    fn a_field_the_cli_does_not_know_about_does_not_shift_the_table() {
        // The server is free to add a column; a script parsing this table must
        // not see the row move underneath it.
        let mut with_extra = item("Same Row", 10);
        with_extra["brand_new_field"] = serde_json::json!({"nested": [1, 2, 3]});
        let before = table(&page(vec![item("Same Row", 10)]));
        let after = table(&page(vec![with_extra]));
        assert_eq!(before, after);
    }

    #[test]
    fn a_long_title_is_truncated_rather_than_pushing_every_column_right() {
        let long = "A".repeat(80);
        let rendered = table(&page(vec![item(&long, 1)]));
        assert!(rendered.contains('…'), "{rendered}");
        // 44 columns for the title, plus the ellipsis replacing the last one.
        assert!(!rendered.contains(&"A".repeat(45)), "{rendered}");
    }

    #[test]
    fn coverage_is_printed_only_when_the_page_carries_it() {
        let plain = page(vec![item("x", 1)]);
        assert_eq!(coverage(&plain), None);

        let with = serde_json::json!({
            "items": [], "fandom": "Naruto", "fandom_tagged_rows": 7626, "catalogue_rows": 141317
        });
        let line = coverage(&with).expect("coverage");
        assert!(line.contains("7626 of 141317"), "{line}");
        assert!(line.contains("5.4%"), "{line}");
    }

    #[test]
    fn the_coverage_line_says_when_the_tagged_rows_cannot_be_ranked() {
        // The live catalogue's shape: 7,626 tagged rows, every one from a source
        // that publishes no endorsements. A page of zeros without this sentence
        // reads as a broken ranking.
        let with = serde_json::json!({
            "items": [], "fandom": "Naruto", "fandom_tagged_rows": 7626,
            "fandom_tagged_with_likes": 0, "catalogue_rows": 141317
        });
        let line = coverage(&with).expect("coverage");
        assert!(line.contains("none of them publishes a like count"), "{line}");

        // When every tagged row can be ranked, the sentence is not printed.
        let rankable = serde_json::json!({
            "items": [], "fandom": "Naruto", "fandom_tagged_rows": 100,
            "fandom_tagged_with_likes": 100, "catalogue_rows": 1000
        });
        let line = coverage(&rankable).expect("coverage");
        assert!(!line.contains("like count"), "{line}");
    }

    #[test]
    fn a_strategy_that_produced_nothing_is_still_printed() {
        let with = serde_json::json!({
            "engine": "rrf", "candidates": 265,
            "strategies": [
                {"name": "cooccurrence", "produced": 0, "matched": 0, "top": []},
                {"name": "time_decay", "produced": 265, "matched": 44,
                 "top": [{"id": "a", "score": 0.0164}]}
            ]
        });
        let block = explanation(&with).expect("explanation");
        assert!(block.contains("cooccurrence"), "the silent strategy is the finding");
        assert!(block.contains("0.0164"), "{block}");
    }

    fn metric_page() -> Value {
        serde_json::json!({
            "items": [
                {"id": "a", "source_key": "sb", "source_work_key": "a", "title": "Measured",
                 "author_text": "Esama", "summary": "", "word_count": 40000, "status": "ongoing",
                 "likes": 33107, "views": null, "source_url": "https://x.invalid",
                 "updated_at": "2026-10-27T00:00:00+00:00", "score": 0.75, "measured": true},
                {"id": "b", "source_key": "sb", "source_work_key": "b", "title": "Unmeasured",
                 "author_text": "Nobody", "summary": "", "word_count": 60000, "status": "ongoing",
                 "likes": 0, "views": null, "source_url": "https://x.invalid",
                 "score": 0.33, "measured": false}
            ],
            "sort": "source_pct", "metric": "source_pct", "ranked": 1247, "total": 141317
        })
    }

    #[test]
    fn a_metric_page_prints_a_score_column_and_an_unmeasured_marker() {
        let rendered = table(&metric_page());
        assert!(rendered.contains("0.750"), "{rendered}");
        assert!(rendered.contains("2026-10-27"), "the source date fills the column: {rendered}");
        // The unmeasured row is marked, so a script cannot mistake triage for rank.
        assert!(rendered.contains('~'), "{rendered}");
    }

    #[test]
    fn the_gate_says_ranked_of_total_and_the_share() {
        let line = gate(&metric_page()).expect("gate");
        assert!(line.contains("1247 of 141317"), "{line}");
        assert!(line.contains("0.9%"), "{line}");
        assert!(line.contains("marked ~"), "{line}");

        // A page where everything was measured needs no caveat.
        let complete = serde_json::json!({
            "items": [], "metric": "source_pct", "ranked": 10, "total": 10
        });
        let line = gate(&complete).expect("gate");
        assert!(!line.contains("marked ~"), "{line}");
    }

    #[test]
    fn the_gate_is_absent_on_a_plain_column_sort() {
        assert_eq!(gate(&page(vec![item("x", 1)])), None);
    }

    #[test]
    fn the_metric_list_names_every_metric_its_status_and_what_unblocks_the_rest() {
        let text = metrics_list();
        for name in ["quality", "source_pct", "engagement_density", "consensus", "crosspost",
                     "words_maturity", "source_coverage", "cohort_pct", "recency", "unmeasured"] {
            assert!(text.contains(name), "{name} missing from:\n{text}");
        }
        assert!(text.contains("velocity*"), "blocked metrics are marked: {text}");
        assert!(text.contains("M63-32"), "and say what unblocks them: {text}");
        assert!(text.contains("raw"), "the stored columns are still there: {text}");
    }

    #[test]
    fn a_sort_word_outside_the_vocabulary_is_refused_locally() {
        assert!(validate_sort("source_pct").is_ok());
        assert!(validate_sort("wilson").is_ok());
        let failure = validate_sort("wilson2").expect_err("unknown");
        assert_eq!(failure.code, 2);
        assert!(failure.message.contains("--list-metrics"));
    }

    #[test]
    fn a_401_says_what_to_do_instead_of_quoting_the_status() {
        let body = serde_json::json!({"error": {"message": "session required"}});
        let failure = failure_for(401, &body, "http://127.0.0.1:8081");
        assert_eq!(failure.code, 2, "retrying unchanged cannot help");
        assert!(failure.message.contains("DARKHOLD_TOKEN"));
    }

    #[test]
    fn a_500_is_retryable_and_a_422_is_not() {
        let body = serde_json::json!({"error": {"message": "fandom: no fandom called \"narut\""}});
        assert_eq!(failure_for(500, &body, "u").code, 1);
        let failure = failure_for(422, &body, "u");
        assert_eq!(failure.code, 2);
        assert!(failure.message.contains("no fandom called"));
    }
}
