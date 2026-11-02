//! `dh module` — what this instance runs, and what it could.
//!
//! The one design rule of this binary (§ above `main`): **every rule lives on the
//! server**. So `list` and `show` ask `/api/v1/modules` and print what came back —
//! the compiled registry in `darkhold-domain` is the *fallback* for an instance
//! that does not answer, and the output says which of the two you are reading.
//! Without that distinction the CLI would be a second source of truth about an
//! instance's posture: an operator running `dh module list` against a host that
//! had switched a surface off would see the compiled default and believe it.
//!
//! `check` is the exception, and deliberately so: it reads a `darkhold.toml` on
//! this machine and validates it with the same function the server runs at
//! startup. A configuration that would refuse to boot should not need a running
//! server to be diagnosed.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use darkhold_domain::modules::{self, ModuleState};
use serde::Deserialize;

use crate::rec::{bad_request, environment, failure_for, Failure};
use crate::Cli;

/// What to do with the module registry.
#[derive(Debug, Clone, Subcommand)]
pub enum ModuleAction {
    /// List every registered surface, its state, and who may change it.
    List {
        /// Only the surfaces this instance runs.
        #[arg(long)]
        enabled: bool,
        /// Only the surfaces this instance does not run.
        #[arg(long)]
        disabled: bool,
        /// Only surfaces that are registered and not built.
        #[arg(long)]
        planned: bool,
    },
    /// One surface in full: its routes, its dependencies, its milestone.
    Show { id: String },
    /// The idea catalogue (§59.7): everything the project has thought about and
    /// not built, most relevant first.
    Ideas {
        /// Hide ideas below this relevance (0..10).
        #[arg(long, default_value_t = 0)]
        min: u8,
        /// Only ideas that came from a requirements row — the committed queue.
        #[arg(long)]
        committed: bool,
    },
    /// Switch a `per_user` surface off for your own account (§59.6).
    ///
    /// The instance's decision is unchanged; only your row is written, and only
    /// you stop seeing it. Needs a session cookie: set DARKHOLD_COOKIE.
    Off {
        /// The module id, as `dh module list` prints it.
        id: String,
    },
    /// Switch it back on for your own account: your row is deleted and the
    /// instance's decision applies again.
    On {
        /// The module id.
        id: String,
    },
    /// Validate a configuration file's `[modules]` table without starting a server.
    ///
    /// This is the check `darkhold serve` runs at startup: an unknown id, a
    /// safety surface, or an id in both lists is a refusal there, and it is the
    /// same refusal here, before you restart anything.
    Check {
        /// The configuration file to check.
        #[arg(long, default_value = "darkhold.toml", value_name = "PATH")]
        config: PathBuf,
    },
}

/// `dh module …`
#[derive(Debug, Args)]
pub struct ModuleArgs {
    /// What to do.
    #[command(subcommand)]
    pub action: ModuleAction,
}

/// Run a module command.
pub async fn run(cli: &Cli, args: ModuleArgs) -> Result<(), Failure> {
    match args.action {
        ModuleAction::List {
            enabled,
            disabled,
            planned,
        } => list(cli, enabled, disabled, planned).await,
        ModuleAction::Show { id } => show(cli, &id).await,
        ModuleAction::Ideas { min, committed } => ideas(cli, min, committed).await,
        ModuleAction::Off { id } => override_one(cli, &id, false).await,
        ModuleAction::On { id } => override_one(cli, &id, true).await,
        ModuleAction::Check { config } => check(&config),
    }
}

/// One row as the server sends it (`GET /api/v1/modules`).
#[derive(Debug, Deserialize)]
struct ServerRow {
    id: String,
    name: String,
    summary: String,
    enabled: bool,
    stability: String,
    toggle: String,
    milestone: String,
    spec: String,
}

#[derive(Debug, Deserialize)]
struct ServerCatalogue {
    allow_user_overrides: bool,
    core: Vec<String>,
    modules: Vec<ServerRow>,
    #[serde(default)]
    ideas: Vec<ServerIdea>,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerIdea {
    id: String,
    name: String,
    relevance: u8,
    source: String,
    milestone: String,
}

/// Fetch the catalogue, or `None` when the instance does not answer.
async fn catalogue(cli: &Cli) -> Result<Option<ServerCatalogue>, Failure> {
    let url = format!("{}/api/v1/modules", cli.base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        // An unreachable instance is not an error for a read a terminal user can
        // still be served from the compiled registry.
        Err(_) => return Ok(None),
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
        return Err(failure_for(status, &body, &cli.base_url));
    }
    let text = response
        .text()
        .await
        .map_err(|error| environment(format!("reading {url}: {error}")))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| environment(format!("{url} did not answer a catalogue: {error}")))
}

fn print_rows(rows: &[&ServerRow], json: bool) -> Result<(), Failure> {
    if json {
        let value: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "name": row.name,
                    "summary": row.summary,
                    "enabled": row.enabled,
                    "stability": row.stability,
                    "toggle": row.toggle,
                    "milestone": row.milestone,
                    "spec": row.spec,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|error| environment(format!("rendering json: {error}")))?
        );
        return Ok(());
    }
    let width = rows.iter().map(|row| row.id.len()).max().unwrap_or(2).max(6);
    for row in rows {
        println!(
            "{:<width$}  {:<4}  {:<12}  {:<9}  {}",
            row.id,
            if row.enabled { "on" } else { "off" },
            row.stability,
            row.toggle,
            row.summary,
            width = width
        );
    }
    Ok(())
}

async fn list(cli: &Cli, enabled: bool, disabled: bool, planned: bool) -> Result<(), Failure> {
    if enabled && disabled {
        return Err(bad_request(
            "--enabled and --disabled ask opposite questions; pick one",
        ));
    }
    let server = catalogue(cli).await?;
    let rows: Vec<ServerRow> = match server {
        Some(catalogue) => {
            if !cli.json {
                eprintln!(
                    "# {} — {} modules, {} core, per-user overrides {}",
                    cli.base_url,
                    catalogue.modules.len(),
                    catalogue.core.len(),
                    if catalogue.allow_user_overrides {
                        "allowed"
                    } else {
                        "refused by the instance"
                    }
                );
            }
            catalogue.modules
        }
        None => {
            if !cli.json {
                eprintln!(
                    "# {} did not answer; showing this build's compiled registry instead. \
                     States below are defaults, not the instance's posture.",
                    cli.base_url
                );
            }
            let defaults = ModuleState::resolve(&[], &[], true);
            modules::all()
                .iter()
                .map(|ext| ServerRow {
                    id: ext.id.to_owned(),
                    name: ext.name.to_owned(),
                    summary: ext.summary.to_owned(),
                    enabled: defaults.enabled(ext.id),
                    stability: ext.stability.as_str().to_owned(),
                    toggle: match ext.toggle {
                        modules::Toggle::Fixed => "fixed".to_owned(),
                        modules::Toggle::Operator => "operator".to_owned(),
                        modules::Toggle::PerUser => "per_user".to_owned(),
                    },
                    milestone: ext.milestone.to_owned(),
                    spec: ext.spec.to_owned(),
                })
                .collect()
        }
    };

    let selected: Vec<&ServerRow> = rows
        .iter()
        .filter(|row| {
            (!enabled || row.enabled)
                && (!disabled || !row.enabled)
                && (!planned || row.stability == "planned")
        })
        .collect();
    print_rows(&selected, cli.json)
}

async fn show(cli: &Cli, id: &str) -> Result<(), Failure> {
    let compiled = modules::find(id).ok_or_else(|| {
        let hint = modules::nearest(id)
            .map(|near| format!("; the nearest registered id is `{near}`"))
            .unwrap_or_default();
        bad_request(format!("no registered module called `{id}`{hint}"))
    })?;

    let server = catalogue(cli).await?;
    // An idea is not a module, and saying "unknown" for one is the CLI confusing
    // two catalogues the API keeps apart.
    let idea = server.as_ref().and_then(|catalogue| {
        catalogue.ideas.iter().find(|row| row.id == id).cloned()
    });
    let state = if let Some(idea) = &idea {
        format!(
            "planned — idea, relevance {} (from {})",
            idea.relevance,
            idea.source.split(',').next().unwrap_or("an unknown document").trim()
        )
    } else {
        match &server {
            Some(catalogue) => catalogue
                .modules
                .iter()
                .find(|row| row.id == id)
                .map_or("unknown (the instance did not list it)".to_owned(), |row| {
                    (if row.enabled { "on" } else { "off" }).to_owned()
                }),
            None => "(instance unreachable; compiled default below)".to_owned(),
        }
    };

    if cli.json {
        let value = serde_json::json!({
            "id": compiled.id,
            "name": compiled.name,
            "summary": compiled.summary,
            "enabled": state,
            "stability": compiled.stability.as_str(),
            "toggle": match compiled.toggle {
                modules::Toggle::Fixed => "fixed",
                modules::Toggle::Operator => "operator",
                modules::Toggle::PerUser => "per_user",
            },
            "requires": compiled.requires,
            "milestone": compiled.milestone,
            "spec": compiled.spec,
            "nav": compiled.nav.iter().map(|n| serde_json::json!({
                "label": n.label, "href": n.href, "slot": n.slot
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|error| environment(format!("rendering json: {error}")))?
        );
        return Ok(());
    }

    println!("{} — {}", compiled.id, compiled.name);
    println!("  state      {state}");
    println!(
        "  stability  {} ({})",
        compiled.stability.as_str(),
        compiled.milestone
    );
    println!(
        "  toggle     {}",
        match compiled.toggle {
            modules::Toggle::Fixed => "fixed — a safety surface, never switchable (§59.4)",
            modules::Toggle::Operator => "operator — the instance decides",
            modules::Toggle::PerUser => "per_user — the instance decides, you may switch it off",
        }
    );
    if !compiled.requires.is_empty() {
        println!("  requires   {}", compiled.requires.join(", "));
    }
    println!("  spec       {}", compiled.spec);
    println!("  relevance  {} / 10", compiled.relevance);
    println!("  source     {}", compiled.source);
    println!("  {}", compiled.summary);
    if !compiled.nav.is_empty() {
        println!("  nav");
        for nav in compiled.nav {
            println!("    [{:<14}] {:<20} {}", nav.slot, nav.label, nav.href);
        }
    }
    Ok(())
}

async fn ideas(cli: &Cli, min: u8, committed: bool) -> Result<(), Failure> {
    let server = catalogue(cli).await?;
    let catalogue = server.ok_or_else(|| {
        environment(
            "the idea catalogue is generated into the build, but this command reads it from the \
             instance so it prints what that instance runs; start one or point --base-url at it",
        )
    })?;
    let rows: Vec<&ServerIdea> = catalogue
        .ideas
        .iter()
        .filter(|idea| idea.relevance >= min)
        .filter(|idea| !committed || idea.source.contains("requirements.csv"))
        .collect();
    if cli.json {
        let value: Vec<serde_json::Value> = rows
            .iter()
            .map(|idea| {
                serde_json::json!({
                    "id": idea.id, "name": idea.name, "relevance": idea.relevance,
                    "milestone": idea.milestone, "source": idea.source,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&value)
                .map_err(|error| environment(format!("rendering json: {error}")))?
        );
        return Ok(());
    }
    eprintln!(
        "# {} ideas on {} at relevance >= {min}{}",
        rows.len(),
        cli.base_url,
        if committed { ", committed only" } else { "" }
    );
    for idea in rows {
        println!(
            "{:>2}  {:<44}  {:<10}  {}",
            idea.relevance,
            idea.id.trim_start_matches("idea."),
            idea.milestone,
            idea.name
        );
    }
    Ok(())
}

/// `dh module off|on <id>` — the reader's own row.
///
/// Sent as a `PATCH` to the same door a browser uses, with the session as a bearer
/// token: there is no second write path, so a bug in the CLI cannot produce a state
/// the web cannot.
async fn override_one(cli: &Cli, id: &str, enabled: bool) -> Result<(), Failure> {
    // A session cookie, not a bearer token. `resolve_bearer` yields an `Actor` with
    // its own scopes and no browser session, so `RequireSession` refuses it — the
    // first version of this command sent a bearer and the live instance answered
    // `AUTH_REQUIRED` for a token that was perfectly valid, which is the kind of
    // 401 that sends you looking at the wrong layer.
    let cookie = cli
        .cookie
        .as_deref()
        .ok_or_else(|| bad_request("changing a surface is per-account: set DARKHOLD_COOKIE"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| environment(format!("http client: {error}")))?;
    let url = format!("{}/api/v1/modules/{id}", cli.base_url);
    let response = client
        .patch(&url)
        .header(reqwest::header::COOKIE, cookie)
        .json(&serde_json::json!({ "enabled": enabled }))
        .send()
        .await
        .map_err(|error| match error.is_connect() {
            true => environment(format!(
                "nothing listening at {} — is the server running? (--base-url)",
                cli.base_url
            )),
            false => environment(format!("request failed: {error}")),
        })?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| environment(format!("reading the response: {error}")))?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    if !(200..300).contains(&status) {
        // The server's refusal is the useful text: it names the toggle that decides,
        // or the milestone that owns the row. Quoting a status instead would hide it.
        let message = body["error"]["message"]
            .as_str()
            .map_or_else(|| format!("HTTP {status}"), |text| text.to_owned());
        return Err(bad_request(message));
    }
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        );
        return Ok(());
    }
    let caller = body["enabled_for_caller"].as_bool().unwrap_or(false);
    let instance = body["instance_enabled"].as_bool().unwrap_or(false);
    println!(
        "{id}: {} for you{}",
        if caller { "on" } else { "off" },
        if !instance {
            " — the instance has it off, so nobody reaches it"
        } else if caller {
            ""
        } else {
            " — your row is written; the instance is unchanged"
        }
    );
    Ok(())
}

/// What a `darkhold.toml` says about modules; every other key is ignored, because
/// checking the whole file is `darkhold serve`'s job and it already refuses.
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    modules: ModulesTable,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModulesTable {
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    enabled: Vec<String>,
    allow_user_overrides: Option<bool>,
}

fn check(path: &PathBuf) -> Result<(), Failure> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        environment(format!(
            "reading {}: {error}. `dh module check` reads the file `darkhold serve` reads",
            path.display()
        ))
    })?;
    // Parsed loosely at the top level: this command is about one table, and a
    // typo in `[server]` is reported by the server with better context than a
    // 6,000-line config struct re-derived here would give.
    let file: ConfigFile = toml::from_str(&text)
        .map_err(|error| bad_request(format!("{}: {error}", path.display())))?;
    match modules::validate(&file.modules.disabled, &file.modules.enabled) {
        Ok(()) => {
            let state = ModuleState::resolve(
                &file.modules.disabled,
                &file.modules.enabled,
                file.modules.allow_user_overrides.unwrap_or(true),
            );
            let on = modules::all()
                .iter()
                .filter(|ext| state.enabled(ext.id))
                .count();
            println!(
                "{}: [modules] is valid — {on} of {} registered surfaces run",
                path.display(),
                modules::all().len()
            );
            Ok(())
        }
        // A refusal is a usage error: the operator's file is wrong (§44's exit
        // vocabulary), not the environment.
        Err(message) => Err(bad_request(message)),
    }
}
