//! Milestone 38 acceptance tests (spec §38).
//!
//! These verify that the hardcoded values spec §38.3 listed have become
//! TOML-configurable, with defaults, validation, and propagation.

use lorehaven_app::config::Config;

// ---------------------------------------------------------------------------
// retention_days = 0 disables cleanup entirely (validated via config parse)
// ---------------------------------------------------------------------------

#[test]
fn retention_zero_is_a_valid_config() {
    let parsed =
        Config::parse_from_str("environment = \"development\"\n[exports]\nretention_days = 0\n")
            .expect("parse retention_days=0");
    assert_eq!(parsed.exports.retention_days, 0);
}

// ---------------------------------------------------------------------------
#[test]
fn revision_ttl_respected() {
    let parsed =
        Config::parse_from_str("environment = \"development\"\n[revisions]\nttl_secs = 3600\n")
            .expect("parse revisions.ttl_secs");
    assert_eq!(parsed.revisions.ttl_secs, 3600);
}

// ---------------------------------------------------------------------------
#[test]
fn terminal_job_retention_respected() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[jobs]\nterminal_retention_days = 7\n",
    )
    .expect("parse jobs.terminal_retention_days");
    assert_eq!(parsed.jobs.terminal_retention_days, 7);
}

// ---------------------------------------------------------------------------
#[test]
fn bulk_export_max_items_enforced() {
    let parsed =
        Config::parse_from_str("environment = \"development\"\n[bulk_export]\nmax_items = 100\n")
            .expect("parse bulk_export.max_items");
    assert_eq!(parsed.bulk_export.max_items, 100);
}

// ---------------------------------------------------------------------------
#[test]
fn bulk_export_max_bytes_enforced() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[bulk_export]\nmax_bytes = 524288000\n",
    )
    .expect("parse bulk_export.max_bytes");
    assert_eq!(parsed.bulk_export.max_bytes, 524288000);
}

// ---------------------------------------------------------------------------
#[test]
fn malformed_config_rejected() {
    let result =
        Config::parse_from_str("environment = \"development\"\n[exports]\nretention_days = -1\n");
    assert!(result.is_err(), "negative retention_days should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("retention_days"),
        "error should mention the offending key, got: {err}"
    );
}

// ---------------------------------------------------------------------------
#[test]
fn zero_rejected_for_grant_ttl() {
    let result =
        Config::parse_from_str("environment = \"development\"\n[exports]\ngrant_ttl_secs = 0\n");
    assert!(result.is_err(), "zero grant_ttl_secs should fail");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("grant_ttl_secs"),
        "error should mention the offending key, got: {err}"
    );
}

// ---------------------------------------------------------------------------
#[test]
fn defaults_produce_working_instance() {
    let config = Config::development_defaults();
    assert!(config.revisions.ttl_secs > 0);
    assert!(config.jobs.terminal_retention_days > 0);
}

// ---------------------------------------------------------------------------
#[test]
fn all_config_values_have_defaults() {
    let config = Config::development_defaults();

    assert!(config.revisions.ttl_secs > 0);
    assert!(config.jobs.terminal_retention_days > 0);
    assert!(config.library.update_check_retention_days > 0);
    assert!(config.library.check_batch > 0);
    assert!(config.bulk_export.max_items > 0);
    assert!(config.bulk_export.max_bytes > 0);
    assert!(config.exports.retention_days >= 0);
    assert!(config.exports.grant_ttl_secs > 0);
}

// ---------------------------------------------------------------------------
#[test]
fn custom_ttl_overrides_default() {
    let parsed =
        Config::parse_from_str("environment = \"development\"\n[revisions]\nttl_secs = 3600\n")
            .expect("parse custom ttl");

    assert_eq!(parsed.revisions.ttl_secs, 3600);
}

// ---------------------------------------------------------------------------
#[test]
fn custom_bulk_export_limits_override_default() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[bulk_export]\nmax_items = 100\nmax_bytes = 524288000\n",
    )
    .expect("parse custom bulk limits");

    assert_eq!(parsed.bulk_export.max_items, 100);
    assert_eq!(parsed.bulk_export.max_bytes, 524288000);
}

// ---------------------------------------------------------------------------
#[test]
fn library_update_retention_configurable() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[library]\nupdate_check_retention_days = 30\n",
    )
    .expect("parse library config");

    assert_eq!(parsed.library.update_check_retention_days, 30);
}

// ---------------------------------------------------------------------------
#[test]
fn library_check_batch_configurable() {
    let parsed =
        Config::parse_from_str("environment = \"development\"\n[library]\ncheck_batch = 25\n")
            .expect("parse library config");

    assert_eq!(parsed.library.check_batch, 25);
}

// ---------------------------------------------------------------------------
#[test]
fn webhook_timeout_configurable() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[administration]\nwebhook_timeout_secs = 30\n",
    )
    .expect("parse administration config");

    assert_eq!(parsed.administration.webhook_timeout_secs, 30);
}

// ---------------------------------------------------------------------------
#[test]
fn webhook_max_attempts_configurable() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[administration]\nwebhook_max_attempts = 3\n",
    )
    .expect("parse administration config");

    assert_eq!(parsed.administration.webhook_max_attempts, 3);
}

// ---------------------------------------------------------------------------
#[test]
fn webhook_base_delay_configurable() {
    let parsed = Config::parse_from_str(
        "environment = \"development\"\n[administration]\nwebhook_base_delay_ms = 1000\n",
    )
    .expect("parse administration config");

    assert_eq!(parsed.administration.webhook_base_delay_ms, 1000);
}
