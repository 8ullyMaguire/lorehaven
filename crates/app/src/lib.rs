//! Lorehaven — composition root.
//!
//! The binary is deliberately thin: this crate owns configuration, logging,
//! startup safety, the HTTP surface and the operational commands. Domain rules
//! live in `lorehaven-domain`; persistence lives in `lorehaven-db`.
//!
//! Architecture decision: a modular monolith with a worker mode, per spec §2.1.
//! One executable means one thing to deploy, back up, and reason about, which
//! matters far more on a single self-hosted machine than the theoretical
//! elasticity of separate services.

pub mod assets;
pub mod auth;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod derivative;
pub mod doctor;
pub mod exports;
pub mod http;
pub mod imports;
pub mod library_updates;
pub mod limiter;
pub mod logging;
pub mod narration;
pub mod privacy;
pub mod revisions;
pub mod routes;
pub mod safety;
pub mod secrets;
pub mod seed;
pub mod server;
pub mod state;
pub mod tts;
pub mod version;
pub mod worker;

use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use lorehaven_db::Database;

use lorehaven_domain::jobs::{JobKind, RetryPolicy};

use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::state::AppState;

/// Parse arguments, load configuration, and dispatch.
pub async fn run(cli: Cli) -> Result<ExitCode> {
    let config = Config::load(&cli.global)?;
    logging::init(&config.logging)?;

    tracing::debug!(
        environment = config.environment.as_str(),
        build = %version::build_id(),
        config_file = ?config.config_path,
        "configuration resolved"
    );

    match cli.command {
        Command::Serve(args) => {
            // Refuse to start a misconfigured production instance *before*
            // opening the database, so the failure cannot have side effects.
            safety::validate_for_startup(&config)?;
            let db = connect(&config).await?;
            server::serve(config, db, &args).await?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Migrate(args) => {
            let db = connect(&config).await?;
            migrate_command(&config, &db, args).await?;
            db.close().await;
            Ok(ExitCode::SUCCESS)
        }

        Command::Seed(args) => {
            let db = connect(&config).await?;
            let summary = seed::run(&config, &db, &args).await?;
            print_seed_summary(&summary, &config);
            db.close().await;
            Ok(ExitCode::SUCCESS)
        }

        Command::Worker(args) => {
            let db = connect(&config).await?;
            let state = AppState::new(config.clone(), db);
            let worker = worker::Worker::new(worker::WorkerOptions::default());
            let shutdown = server::shutdown_signal();
            if args.once {
                let report = worker.run_once(&state).await?;
                print_pass_report(&report);
            } else {
                tracing::info!(worker = %worker.options().id, "worker started");
                worker.run(&state, shutdown).await?;
            }
            state.db().close().await;
            Ok(ExitCode::SUCCESS)
        }

        Command::Maintain(args) => {
            let db = connect(&config).await?;
            for task in MAINTENANCE_TASKS {
                if args.dry_run {
                    println!("would queue {task}");
                    continue;
                }
                let job_id = lorehaven_db::jobs::enqueue(
                    &db,
                    JobKind::Maintenance,
                    &serde_json::json!({ "task": task }).to_string(),
                    None,
                    None,
                    0,
                    &RetryPolicy::default(),
                )
                .await?;
                println!("queued {task} as {job_id}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor(args) => {
            let report = doctor::run(&config, &args).await;
            print!("{}", doctor::render(&report));
            let failed = report.has_failures() || (args.strict && report.has_warnings());
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

/// The recurring maintenance tasks, in the order they are queued.
///
/// A list rather than a match arm so the CLI and the worker's task names cannot
/// drift: this is the only place a task name is written for scheduling, and the
/// worker's own `match` refuses anything it does not know.
pub const MAINTENANCE_TASKS: [&str; 3] = ["reap_jobs", "collect_blobs", "purge_exports"];

/// What one `worker --once` pass did, for an operator watching the queue move.
fn print_pass_report(report: &worker::PassReport) {
    println!(
        "outbox: {} delivered, {} failed, {} deferred (no handler in this build)",
        report.outbox_delivered, report.outbox_failed, report.outbox_deferred
    );
    match &report.job {
        Some((id, state)) => println!("job {id}: {}", state.as_str()),
        None => println!("job: none was waiting"),
    }
}

/// Open the database configured for this instance.
pub async fn connect(config: &Config) -> Result<Database> {
    Database::connect(&config.database).await.with_context(|| {
        format!(
            "connecting to {} as {}",
            config.database.url,
            config.environment.as_str()
        )
    })
}

async fn migrate_command(
    config: &Config,
    db: &Database,
    args: crate::cli::MigrateArgs,
) -> Result<()> {
    use lorehaven_db::migrate;

    let backend = db.backend();
    let known = migrate::catalogue(backend);

    if args.status {
        let applied = migrate::applied(db).await?;
        let pending = migrate::pending(db).await?;

        println!(
            "backend:   {}\nurl:       {}\ncompiled:  {} migration(s)\n",
            backend.as_str(),
            config.database.url,
            known.len()
        );

        for migration in known {
            let id = migration.id();
            match applied.iter().find(|row| row.id == id) {
                Some(row) => println!("  applied  {id}  ({})", row.applied_at),
                None => println!("  pending  {id}"),
            }
        }

        println!("\n{} applied, {} pending", applied.len(), pending.len());
        return Ok(());
    }

    let report = db.migrate().await?;
    if report.applied.is_empty() {
        println!(
            "schema already up to date: {} migration(s) applied on this {} database",
            report.already_applied.len(),
            report.backend.as_str()
        );
    } else {
        println!(
            "applied {} migration(s) to the {} database:",
            report.applied.len(),
            report.backend.as_str()
        );
        for id in &report.applied {
            println!("  {id}");
        }
    }
    Ok(())
}

fn print_seed_summary(summary: &seed::SeedSummary, config: &Config) {
    println!(
        "Development data ready in the {} environment.",
        config.environment.as_str()
    );
    if summary.reset {
        println!("(previous development identity data was removed first)");
    }
    println!("\n  sign in as:  {}", summary.email);
    println!("  password:    {}", summary.password);
    println!("  account id:  {}", summary.account_id);
    println!("\n  pseuds:");
    for (handle, id) in &summary.pseuds {
        println!("    @{handle}  ({id})");
    }
    println!(
        "\nThis account is a development fixture. Never run `seed` against an \
         instance holding real content."
    );
}

/// Entry point used by `main.rs`.
pub async fn main_entry() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(error) => {
            // `{error:#}` prints the whole anyhow chain: the operator needs the
            // root cause, not just "connection failed".
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// An instant in RFC 3339 UTC, which is the only timestamp format stored.
///
/// One function because the format is a storage contract: two modules that
/// format the same instant differently produce rows that compare wrongly and
/// cursors that skip pages.
#[must_use]
pub fn format_rfc3339(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| at.to_string())
}
