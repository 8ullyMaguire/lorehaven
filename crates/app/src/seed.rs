//! Development seeding.
//!
//! Spec §5 requires a development seed command. Two rules make it safe:
//!
//! 1. It refuses to run in production, *and* refuses when `[dev] seed_enabled`
//!    is false, so a production instance cannot be seeded even if the flag is
//!    passed by mistake.
//! 2. It is idempotent. Running it twice leaves one account and two pseuds, not
//!    two of each — a seed command that multiplies rows makes every later
//!    manual test untrustworthy.

use anyhow::{bail, Result};
use lorehaven_db::identity::{self, AccountStatus, PrivacyScope};
use lorehaven_db::Database;
use lorehaven_domain::policy::AgeState;
use lorehaven_domain::{AccountId, PseudId};

use crate::cli::SeedArgs;
use crate::config::Config;
use crate::crypto;

/// What the seed run produced.
#[derive(Debug)]
pub struct SeedSummary {
    /// The development account.
    pub account_id: AccountId,
    /// Its sign-in address.
    pub email: String,
    /// The password that was set (echoed for the developer).
    pub password: String,
    /// Pseud handles now available, with their identifiers.
    pub pseuds: Vec<(String, PseudId)>,
    /// Whether existing development data was removed first.
    pub reset: bool,
}

/// Create development data.
pub async fn run(config: &Config, db: &Database, args: &SeedArgs) -> Result<SeedSummary> {
    if !args.development {
        bail!(
            "`seed` refuses to run without --development: it writes a known \
             password and must never touch a real instance"
        );
    }
    if config.environment.is_production() {
        bail!(
            "`seed` refuses to run against the production environment \
             (LOREHAVEN_ENV=production)"
        );
    }
    if !config.dev.seed_enabled {
        bail!("`seed` is disabled by configuration ([dev] seed_enabled = false)");
    }

    // The schema must exist before we write to it.
    let pending = lorehaven_db::migrate::pending(db).await?;
    if !pending.is_empty() {
        bail!(
            "{} migration(s) are pending; run `lorehaven migrate` first",
            pending.len()
        );
    }

    if args.reset {
        identity::wipe_identity(db).await?;
        tracing::info!("existing development identity data removed");
    }

    // --- account ------------------------------------------------------------
    let account_id = match identity::find_account_by_email(db, &args.email).await? {
        Some(account) => account.id,
        None => {
            let id = identity::create_account(
                db,
                &args.email,
                AgeState::DeclaredAdult,
                AccountStatus::Active,
            )
            .await?;
            tracing::info!(email = %args.email, "created development account");
            id
        }
    };

    let hash = crypto::hash_password(&args.password)?;
    identity::set_password_hash(db, account_id, &hash).await?;

    // --- pseuds -------------------------------------------------------------
    let desired = [("devwriter", "Dev Writer"), ("devreader", "Dev Reader")];
    let mut pseuds = Vec::new();

    for (handle, display_name) in desired {
        let id = match identity::find_pseud_by_handle(db, handle).await? {
            Some(existing) => {
                if existing.account_id != account_id {
                    bail!(
                        "handle @{handle} already belongs to another account; \
                         run with --reset or choose different handles"
                    );
                }
                existing.id
            }
            None => {
                let id = identity::create_pseud(db, account_id, handle, display_name).await?;
                tracing::info!(handle, "created development pseud");
                id
            }
        };
        pseuds.push((handle.to_owned(), id));
    }

    // --- privacy defaults ---------------------------------------------------
    // Spec §7: protection defaults are stored from onboarding rather than
    // assumed at render time. These are the defaults a new account gets.
    identity::set_privacy(
        db,
        PrivacyScope::Account(&account_id),
        "messaging_policy",
        "contacts_only",
    )
    .await?;
    identity::set_privacy(
        db,
        PrivacyScope::Account(&account_id),
        "recommendation_opt_in",
        "true",
    )
    .await?;

    for (_, pseud_id) in &pseuds {
        identity::set_privacy(
            db,
            PrivacyScope::Pseud(pseud_id),
            "public_bookmarks",
            "private",
        )
        .await?;
    }

    Ok(SeedSummary {
        account_id,
        email: args.email.clone(),
        password: args.password.clone(),
        pseuds,
        reset: args.reset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(development: bool) -> SeedArgs {
        SeedArgs {
            development,
            reset: false,
            email: "dev@lorehaven.local".to_owned(),
            password: "lorehaven-dev".to_owned(),
        }
    }

    /// A database in a fresh temporary directory, so the guard tests exercise
    /// real connections without depending on the host's filesystem layout.
    async fn scratch_database(tag: &str) -> (Database, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lorehaven-seed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let url = format!("sqlite://{}/seed.sqlite?mode=rwc", dir.display());
        let db = Database::connect(&lorehaven_db::DatabaseConfig::new(url))
            .await
            .expect("connect");
        (db, dir)
    }

    #[tokio::test]
    async fn seeding_without_the_flag_is_refused() {
        let config = Config::development_defaults();
        let (db, dir) = scratch_database("flag").await;
        let error = run(&config, &db, &args(false))
            .await
            .expect_err("must refuse");
        assert!(format!("{error}").contains("--development"));
        db.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn seeding_is_refused_in_production() {
        let mut config = Config::development_defaults();
        config.environment = crate::config::Environment::Production;
        let (db, dir) = scratch_database("prod").await;
        let error = run(&config, &db, &args(true))
            .await
            .expect_err("must refuse");
        assert!(format!("{error}").contains("production"));
        db.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn seeding_is_refused_when_disabled_in_configuration() {
        let mut config = Config::development_defaults();
        config.dev.seed_enabled = false;
        let (db, dir) = scratch_database("disabled").await;
        let error = run(&config, &db, &args(true))
            .await
            .expect_err("must refuse");
        assert!(format!("{error}").contains("seed_enabled"));
        db.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn seeding_requires_an_up_to_date_schema() {
        // A guard that matters: writing fixtures into a stale schema would
        // produce rows the running binary cannot read.
        let config = Config::development_defaults();
        let (db, dir) = scratch_database("schema").await;
        let error = run(&config, &db, &args(true))
            .await
            .expect_err("must refuse");
        assert!(format!("{error}").contains("pending"), "{error}");
        db.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }
}
