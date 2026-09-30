//! Acceptance: `[roadmap]` and `[retention_governance]` are loadable from a
//! file (config-reference, M59 Phase E).
//!
//! This file exists because of a defect it would have caught. Both settings were
//! fields on `Config` with defaults wired into `development_defaults()` and no
//! `FileConfig` member: a Rust test could set them, and every test that proved
//! "the widening bar is read from configuration" passed — while an operator
//! editing `lorehaven.toml` found the key silently ignored. A setting that
//! reads as configurable and is not is worse than one that is absent, because
//! the tests read as evidence it works. The `[retention]` section's own doc
//! comment in `config.rs` describes this exact trap having already happened.
//!
//! So: a real file, loaded through the real loader, with values that differ from
//! every default. And the negative case, because a misspelled governance key is
//! the one that must be an error rather than a setting that does not apply.

use lorehaven_app::cli::GlobalArgs;
use lorehaven_app::config::Config;
use std::path::{Path, PathBuf};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-config-sections-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write `body` as `lorehaven.toml` and load it the way the binary does.
///
/// Through `GlobalArgs { config: Some(..) }` rather than by reading the file
/// directly: `--config` is the door a deployment comes through, and a test that
/// used some other route would prove the struct deserialises without proving the
/// binary's own path reaches it — which is the shape of the bug this file exists
/// to catch.
fn load(tag: &str, body: &str) -> Result<Config, String> {
    let dir = scratch_dir(tag);
    let path = dir.join("lorehaven.toml");
    std::fs::write(&path, body).expect("write config");
    let args = GlobalArgs {
        config: Some(path),
        ..GlobalArgs::default()
    };
    Config::load(&args).map_err(|error| error.to_string())
}

const FULL: &str = r#"
[roadmap]
min_trust = 4

[retention_governance]
proposal_min_trust = 3
widen_quorum = 7
proposal_cooling_days = 21
binding_mode = true
"#;

#[test]
fn the_roadmap_and_retention_governance_sections_are_read_from_the_file() {
    let config = load("full", FULL).expect("a valid file loads");

    assert_eq!(
        config.roadmap.min_trust, 4,
        "roadmap.min_trust came from the file, not the default of 1"
    );
    assert_eq!(config.retention_governance.proposal_min_trust, 3);
    assert_eq!(config.retention_governance.widen_quorum, 7);
    assert_eq!(config.retention_governance.proposal_cooling_days, 21);
    assert!(
        config.retention_governance.binding_mode,
        "binding_mode is a bool in the file and a bool on the struct"
    );
}

#[test]
fn a_file_naming_one_key_gets_the_documented_default_for_the_rest() {
    // Partial sections are the common case: an operator turning on binding mode
    // should not have to restate the cooling period. And the defaults must be
    // the ones `Default` says, not a second copy of them in the parser.
    let config = load("partial", "[retention_governance]\nbinding_mode = true\n")
        .expect("a partial section loads");

    assert!(config.retention_governance.binding_mode);
    assert_eq!(config.retention_governance.proposal_min_trust, 1);
    assert_eq!(config.retention_governance.widen_quorum, 3);
    assert_eq!(config.retention_governance.proposal_cooling_days, 7);
    assert_eq!(
        config.roadmap.min_trust, 1,
        "an absent section is the default, not zero"
    );
}

#[test]
fn a_misspelled_governance_key_is_refused_at_load_rather_than_ignored() {
    // The failure mode this refuses: an operator writes `widen_quorum = 7`, an
    // operator writes `wide_quorum = 7` by mistake, the instance loads, and the
    // governance bar is 3 — a setting that reads as applied and is not.
    for (tag, body) in [
        ("typo-widen", "[retention_governance]\nwide_quorum = 7\n"),
        (
            "typo-proposal",
            "[retention_governance]\nproposal_min_trustt = 3\n",
        ),
        ("typo-roadmap", "[roadmap]\nmin_trust_level = 4\n"),
    ] {
        // Loaded against the `anyhow::Error` itself rather than through `load`'s
        // `String`, because the `unknown field` line is one level down the source
        // chain: `Display` on an `anyhow::Error` is only the outermost context
        // (`parsing <path>`) and does not walk it. Matching the first line alone
        // would pass against a refusal that never named the key, which is the
        // whole thing being checked — and `Error::source` is unavailable because
        // `anyhow::Error` deliberately does not implement `std::error::Error`.
        let path = scratch_dir(tag).join("lorehaven.toml");
        std::fs::write(&path, body).expect("write config");
        let args = GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        };
        let error: anyhow::Error = match Config::load(&args) {
            Ok(_) => panic!("{tag}: a misspelled key must be refused"),
            Err(error) => error,
        };

        // Matching the first line alone would pass against a refusal that never
        // named the key, which is the whole thing being checked — so the whole
        // chain is walked. `anyhow::Error::chain()` is the supported way to do
        // this (`anyhow::Error` deliberately does not implement
        // `std::error::Error`, so `Error::source` is not available on it).
        let text = error
            .chain()
            .map(|link| link.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("unknown field"),
            "{tag}: the refusal names the unknown key: {text}"
        );
    }
}

#[test]
fn the_body_request_bar_is_read_from_the_file() {
    // §11.15b's bar, through the FILE path and not the struct. The defect this
    // pins is `be68f32`: `roadmap.min_trust` and four `retention_governance`
    // values had a `Config` field and no `FileConfig` member, so every Rust test
    // set them and passed while an operator's TOML key was silently ignored. A
    // test that constructs the struct cannot catch that; only this one can.
    let config = load(
        "body-request-bar",
        "[retention]\nbody_request_min_trust = 5\n",
    )
    .expect("a valid file loads");

    assert_eq!(
        config.retention.body_request_min_trust, 5,
        "retention.body_request_min_trust came from the file, not the default of 2"
    );
}

#[test]
fn a_file_naming_one_retention_key_keeps_the_documented_default_for_the_bar() {
    // The common case, and the reason the load site carries its own
    // `unwrap_or(2)` rather than borrowing `RetentionConfig::default()`: an
    // operator setting `default_body_audience` should not have to restate the
    // bar, and the value they get must be the one §6.3 documents.
    let config = load(
        "retention-partial",
        "[retention]\ndefault_body_audience = \"Anyone\"\n",
    )
    .expect("a partial section loads");

    assert_eq!(
        config.retention.body_request_min_trust, 2,
        "an absent key is the documented default of 2, not zero and not the last one set"
    );
}

#[test]
fn no_read_path_consults_the_request_bar() {
    // §6.4.4: "no route, surface or rendered page varies in whether a body is
    // shown according to the viewer's trust level, and a test fails the build if
    // one appears."
    //
    // A SCAN, not a behavioural test, and deliberately. A behavioural test can
    // only show that the readers it exercises agree; the property is about code
    // that does not exist yet, and only a scan constrains it. The plan says the
    // guard is written last for the same reason — until the route exists it has
    // nothing to be true about.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let needle = "body_request_min_trust";

    // The four places the value may legitimately appear.
    let allowed = [
        "crates/app/src/config.rs", // the struct, the section, the load site
        "crates/app/src/routes/reader_body_copies.rs", // the gate that checks it
    ];

    let mut violations = Vec::new();
    for entry in std::fs::read_dir(repo.join("crates")).expect("read crates/") {
        let dir = entry.expect("dir entry").path();
        if !dir.is_dir() {
            continue;
        }
        for rs in collect_rs(&dir) {
            let rel = rs
                .strip_prefix(&repo)
                .expect("under the repo")
                .to_string_lossy()
                .to_string();
            if allowed.contains(&rel.as_str()) || rel.contains("tests/") {
                continue;
            }
            let text = std::fs::read_to_string(&rs).expect("read source");
            if text.contains(needle) {
                for (n, line) in text.lines().enumerate() {
                    if line.contains(needle) {
                        violations.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "§6.4.4: a read path must not branch on the viewer's trust to decide \
         whether a body is shown. The request bar gates the REQUEST; once a copy \
         exists every reader eligible for the work reads the same thing.\n{}",
        violations.join("\n")
    );
}

#[test]
fn an_absent_file_keeps_every_default() {
    // The no-file path, which is what the development defaults and every test
    // harness use. If this drifted, every existing instance would change
    // behaviour on upgrade, which is the constraint the plan states.
    let config = Config::development_defaults();
    assert_eq!(config.roadmap.min_trust, 1);
    assert_eq!(config.retention_governance.proposal_min_trust, 1);
    assert_eq!(config.retention_governance.widen_quorum, 3);
    assert_eq!(config.retention_governance.proposal_cooling_days, 7);
    assert!(!config.retention_governance.binding_mode);
    assert_eq!(
        config.retention.body_request_min_trust, 2,
        "§6.3's default of 2 applies with no file at all"
    );
}

/// Every `.rs` under `dir`, so the §6.4.4 scan sees the whole crate and not
/// just the files a reader would think to check.
fn collect_rs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
