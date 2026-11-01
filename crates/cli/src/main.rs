//! `dh` has one job so far: take a question a power user asks at a terminal
//! ("naruto fics updated last week, by likes, using the rrf engine, and show me
//! why") and answer it without a browser.
//!
//! # The one design rule
//!
//! **Every rule lives on the server, and this binary repeats none of them.** The
//! window grammar, the metric vocabulary, the enabled engines, the suggestion for
//! a misspelt fandom — all of it is resolved by `/api/v1`. The CLI sends what was
//! typed and prints what came back, so a window typed here cannot mean something
//! different from the same window in a URL. That is also why `--sort wilson`
//! fails today with the server's own message rather than being mapped to `likes`:
//! a client-side alias table is a second definition waiting to disagree.

mod rec;

use clap::{Parser, Subcommand};

/// Query a Darkhold catalogue from a terminal.
#[derive(Debug, Parser)]
#[command(
    name = "dh",
    version,
    about = "Query a Darkhold catalogue from a terminal",
    long_about = None,
    propagate_version = true
)]
pub struct Cli {
    /// Base URL of the instance to talk to.
    #[arg(
        long,
        global = true,
        env = "DARKHOLD_BASE_URL",
        default_value = "http://127.0.0.1:8081",
        value_name = "URL"
    )]
    pub base_url: String,

    /// Session token, for the surfaces that read your own history.
    ///
    /// Only `--engine` needs it: the recommendation engines rank against what
    /// this account has already read, so they are not a public route. Without it
    /// a plain catalogue query still works.
    #[arg(long, global = true, env = "DARKHOLD_TOKEN", value_name = "TOKEN")]
    pub token: Option<String>,

    /// Print the server's JSON verbatim instead of a table.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// What to do.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Recommend from the catalogue: a fandom, a window, a metric, an engine.
    ///
    /// The window grammar is the server's, not this program's:
    ///
    /// ```text
    ///   7d                        the last seven days
    ///   >30d                      older than thirty days
    ///   @2026-10-27               that whole day, UTC
    ///   2026-01-01..2026-10-15    between two days
    /// ```
    Rec(rec::RecArgs),
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match &cli.command {
        Command::Rec(args) => match rec::run(&cli, args).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::from(error.code)
            }
        },
    }
}
