//! Acceptance: `[roadmap]` and `[retention_governance]` are loadable from a
//! file (config-reference, M59 Phase E).
//!
//! This file exists because of a defect it would have caught. Both settings were
//! fields on `Config` with defaults wired into `development_defaults()` and no
//! `FileConfig` member: a Rust test could set them, and every test that proved
//! "the widening bar is read from configuration" passed — while an operator
//! editing `darkhold.toml` found the key silently ignored. A setting that
//! reads as configurable and is not is worse than one that is absent, because
//! the tests read as evidence it works. The `[retention]` section's own doc
//! comment in `config.rs` describes this exact trap having already happened.
//!
//! So: a real file, loaded through the real loader, with values that differ from
//! every default. And the negative case, because a misspelled governance key is
//! the one that must be an error rather than a setting that does not apply.

use darkhold_app::cli::GlobalArgs;
use darkhold_app::config::Config;
use std::path::{Path, PathBuf};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "darkhold-config-sections-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write `body` as `darkhold.toml` and load it the way the binary does.
///
/// Through `GlobalArgs { config: Some(..) }` rather than by reading the file
/// directly: `--config` is the door a deployment comes through, and a test that
/// used some other route would prove the struct deserialises without proving the
/// binary's own path reaches it — which is the shape of the bug this file exists
/// to catch.
fn load(tag: &str, body: &str) -> Result<Config, String> {
    let dir = scratch_dir(tag);
    let path = dir.join("darkhold.toml");
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
        let path = scratch_dir(tag).join("darkhold.toml");
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

// ---------------------------------------------------------------------------
// The sections M1 added (spec §12.6, §12.10, §12.12, §14.1, §14.3, §14.4, §14.5,
// §18, §9.5, §12.3, §12.1, §8.2).
//
// Each section gets the same three tests, because each section has the same three
// failure modes:
//
//   * it PARSES, and the value reaches the struct — the defect this file exists
//     for, where a setting had a `Config` field and no `FileConfig` member, so
//     every Rust test set it and passed while the operator's TOML key was ignored;
//   * an UNKNOWN key is refused at load — a misspelled governance setting that
//     reads as applied and is not;
//   * an OUT-OF-RANGE value is refused at load — a value that cannot be sane in
//     any environment, and that must be caught where the operator can see it
//     rather than in the middle of an ordinary request.
// ---------------------------------------------------------------------------

/// A section's valid fragment, a fragment with a misspelled key in it, and a
/// fragment with a value that cannot be sane.
struct SectionCase {
    /// A short name, used for the scratch directory.
    name: &'static str,
    /// A fragment that must load.
    valid: &'static str,
    /// A complete fragment with one key that does not exist. It has to be a
    /// *complete* fragment and not a key appended to `valid`, because a key
    /// outside a table is a TOML syntax error rather than an unknown field --
    /// and a syntax error would make this test pass for the wrong reason.
    unknown_key: &'static str,
    /// A fragment whose value is out of range.
    out_of_range: &'static str,
    /// The key the out-of-range fragment names, so the refusal can be checked to
    /// name it.
    range_key: &'static str,
}

/// Every section M1 added, with its three fragments.
fn section_cases() -> Vec<SectionCase> {
    vec![
        SectionCase {
            name: "tiers",
            valid: "[tiers]\nhouse = \"always\"\npersonal_min_trust = 2\nfull_supporter = false\nfull_min_trust = 5\nfull_min_resonance = 0.4\n",
            unknown_key: "[tiers]\npersonal_min_trustt = 2\n",
            out_of_range: "[tiers]\nfull_min_resonance = 1.5\n",
            range_key: "tiers.full_min_resonance",
        },
        SectionCase {
            name: "channels",
            valid: "[channels.voting]\nenabled = false\nmin_trust_level = 4\nmin_resonance = 0.7\nsupporter_only = true\nperiod_days = 14\nmax_shift_per_period = 0.1\nseed_floor = 0.4\ntau = 30.0\n",
            unknown_key: "[channels.voting]\nperiod_day = 14\n",
            out_of_range: "[channels.voting]\nperiod_days = 0\n",
            range_key: "channels.voting.period_days",
        },
        SectionCase {
            name: "supporters",
            valid: "[supporters.grants]\nself_hosted = true\npayment_processor = true\nfederated = true\ncode_purchase = true\n\n[supporters.price]\nmonthly = 700\nannual = 7000\ncurrency = \"USD\"\n",
            unknown_key: "[supporters.price]\nmontly = 700\n",
            out_of_range: "[supporters.price]\nmonthly = -1\n",
            range_key: "supporters.price.monthly",
        },
        SectionCase {
            name: "consent",
            valid: "[consent]\nrequired_for_guest = false\nversion = 4\nage_gate = \"optional\"\nreask_on_version_change = false\n\n[consent.categories]\nessential = true\nanalytics = true\n",
            unknown_key: "[consent]\nreask_on_version_chang = false\n",
            out_of_range: "[consent]\nversion = 0\n",
            range_key: "consent.version",
        },
        SectionCase {
            name: "guest",
            valid: "[guest]\nenabled = false\nmau_limit = 1000\nmau_window_days = 14\non_cap = \"throttle\"\ngrace_days = 3\nparity_level = 2\n",
            unknown_key: "[guest]\nmau_limt = 1000\n",
            out_of_range: "[guest]\nparity_level = 99\n",
            range_key: "guest.parity_level",
        },
        SectionCase {
            name: "merge",
            valid: "[merge]\nenabled = false\ngrants_trust = true\ngrants_tenure = false\nnotify = false\ntombstone = \"delete\"\n",
            unknown_key: "[merge]\ngrants_trus = true\n",
            out_of_range: "[merge]\ntombstone = \"obliterate\"\n",
            range_key: "merge.tombstone",
        },
        SectionCase {
            name: "trust",
            valid: "[trust]\nelastic_min_holders = 3\nfloor_level = 1\nelastic_decay_days = 14\nguest_level = 2\nguest_earns = false\n",
            unknown_key: "[trust]\nfloor_levell = 1\n",
            out_of_range: "[trust]\nfloor_level = 42\n",
            range_key: "trust.floor_level",
        },
        SectionCase {
            name: "governance",
            valid: "[governance]\nenabled = false\njury_size = 7\njury_min_level = 4\njury_elastic_min = 5\nappeal_fresh_jury = false\nadmin_emergency_only = false\nban_review_days = 30\n",
            unknown_key: "[governance]\njury_siz = 7\n",
            out_of_range: "[governance]\njury_size = 0\n",
            range_key: "governance.jury_size",
        },
        SectionCase {
            name: "discovery-moods",
            valid: "[discovery.moods]\nenabled = false\nmin_trust_level = 4\nsupporter_only = true\nmax_moods_per_account = 3\n",
            unknown_key: "[discovery.moods]\nmax_moods_per_accont = 3\n",
            out_of_range: "[discovery.moods]\nmax_moods_per_account = 0\n",
            range_key: "discovery.moods.max_moods_per_account",
        },
        SectionCase {
            name: "theme",
            valid: "[theme]\npublished = true\nopposition_weight = 0.5\n\n[theme.signal_weights]\nworks = 0.5\nauthors = 0.3\ntags = 0.2\n",
            unknown_key: "[theme]\nopposition_weigth = 0.5\n",
            out_of_range: "[theme]\nopposition_weight = 2.0\n",
            range_key: "theme.opposition_weight",
        },
        SectionCase {
            name: "taste",
            valid: "[taste]\nmode = \"service\"\n",
            unknown_key: "[taste]\nmdoe = \"service\"\n",
            out_of_range: "[taste]\nmode = \"sideways\"\n",
            range_key: "taste.mode",
        },
    ]
}

#[test]
fn every_new_section_is_read_from_the_file() {
    // The positive case, and the one this file exists for: the value has to
    // arrive through the loader, not through a struct the test built.
    for case in section_cases() {
        let config = load(&format!("valid-{}", case.name), case.valid)
            .unwrap_or_else(|error| panic!("{}: a valid file loads: {error}", case.name));

        // Every case names at least one key, and each key has to be somewhere in
        // the loaded configuration. The check is per-section and explicit, because
        // a generic "the config changed" assertion would pass against a section
        // that parsed and was ignored.
        match case.name {
            "tiers" => {
                assert_eq!(config.tiers.personal_min_trust, 2, "tiers");
                assert_eq!(config.tiers.full_min_trust, 5, "tiers");
                assert!(!config.tiers.full_supporter, "tiers");
            }
            "channels" => {
                assert_eq!(config.channels.min_trust_level, 4, "channels");
                assert_eq!(config.channels.period_days, 14, "channels");
                assert_eq!(config.channels.tau, 30.0, "channels");
                assert!(!config.channels.voting_enabled, "channels");
            }
            "supporters" => {
                assert_eq!(config.supporters.price.monthly, 700, "supporters");
                assert_eq!(config.supporters.price.currency, "USD", "supporters");
                assert!(config.supporters.grants.federated, "supporters");
            }
            "consent" => {
                assert_eq!(config.consent.version, 4, "consent");
                assert_eq!(config.consent.age_gate, "optional", "consent");
                assert!(
                    config.consent.categories.get("analytics") == Some(&true),
                    "consent: the categories table came from the file"
                );
            }
            "guest" => {
                assert_eq!(config.guest.mau_limit, 1000, "guest");
                assert_eq!(config.guest.on_cap, "throttle", "guest");
                assert!(!config.guest.enabled, "guest");
            }
            "merge" => {
                assert!(config.merge.grants_trust, "merge");
                assert!(!config.merge.grants_tenure, "merge");
                assert_eq!(config.merge.tombstone, "delete", "merge");
            }
            "trust" => {
                assert_eq!(config.trust.elastic_min_holders, 3, "trust");
                assert_eq!(config.trust.floor_level, 1, "trust");
                assert!(!config.trust.guest_earns, "trust");
            }
            "governance" => {
                assert_eq!(config.governance.jury_size, 7, "governance");
                assert_eq!(config.governance.ban_review_days, 30, "governance");
                assert!(!config.governance.appeal_fresh_jury, "governance");
            }
            "discovery-moods" => {
                assert_eq!(config.discovery.moods.max_moods_per_account, 3, "moods");
                assert_eq!(config.discovery.moods.min_trust_level, 4, "moods");
                assert!(config.discovery.moods.supporter_only, "moods");
            }
            "theme" => {
                assert!(config.theme.published, "theme");
                assert_eq!(config.theme.opposition_weight, 0.5, "theme");
                assert_eq!(config.theme.signal_weights.works, 0.5, "theme");
                assert_eq!(config.theme.signal_weights.authors, 0.3, "theme");
                assert_eq!(config.theme.signal_weights.tags, 0.2, "theme");
            }
            "taste" => {
                assert_eq!(config.taste.mode, "service", "taste");
            }
            other => panic!("{other}: the section case has no assertion for it"),
        }
    }
}

#[test]
fn a_misspelled_key_in_any_new_section_is_refused_at_load() {
    // A setting that reads as applied and is not. The whole chain is walked, so
    // the refusal has to name the key rather than merely refusing.
    for case in section_cases() {
        let body = case.unknown_key.to_string();
        let path = scratch_dir(&format!("typo-{}", case.name)).join("darkhold.toml");
        std::fs::write(&path, body).expect("write config");
        let args = GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        };
        let error: anyhow::Error = match Config::load(&args) {
            Ok(_) => panic!("{}: a misspelled key must be refused", case.name),
            Err(error) => error,
        };
        let text = error
            .chain()
            .map(|link| link.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("unknown field"),
            "{}: the refusal names the unknown key: {text}",
            case.name
        );
    }
}

#[test]
fn an_out_of_range_value_in_any_new_section_is_refused_at_load() {
    // **Validation at startup, not at use** (spec §19 rule 3). A value that cannot
    // be sane must be refused where the operator can see it, and the refusal has
    // to NAME the key — a refusal that does not name the key is a refusal the
    // operator cannot act on.
    for case in section_cases() {
        let path = scratch_dir(&format!("range-{}", case.name)).join("darkhold.toml");
        std::fs::write(&path, case.out_of_range).expect("write config");
        let args = GlobalArgs {
            config: Some(path),
            ..GlobalArgs::default()
        };
        let error: anyhow::Error = match Config::load(&args) {
            Ok(_) => panic!(
                "{}: an out-of-range value must be refused: {}",
                case.name, case.out_of_range
            ),
            Err(error) => error,
        };
        let text = error
            .chain()
            .map(|link| link.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(case.range_key),
            "{}: the refusal names the key ({}): {text}",
            case.name,
            case.range_key
        );
    }
}

#[test]
fn a_partial_section_keeps_the_documented_default_for_every_key_it_omits() {
    // The common case: an operator turning on one knob should not have to restate
    // the rest, and the values they get must be the documented ones.
    let config = load(
        "partial-new-sections",
        "[tiers]\nfull_min_trust = 5\n\n[guest]\nmau_limit = 100\n",
    )
    .expect("a partial file loads");

    assert_eq!(config.tiers.full_min_trust, 5, "the key that was named");
    assert_eq!(
        config.tiers.personal_min_trust, 1,
        "an absent key is the documented default, not zero"
    );
    assert_eq!(
        config.tiers.full_min_resonance, 0.55,
        "an absent key is the documented default"
    );
    assert_eq!(config.guest.mau_limit, 100, "the key that was named");
    assert_eq!(
        config.guest.mau_window_days, 30,
        "an absent key is the documented default"
    );
    assert_eq!(
        config.guest.parity_level, 1,
        "an absent key is the documented default"
    );
}

#[test]
fn the_theme_signal_weights_must_keep_their_order() {
    // **The ordering is the design and the numbers are configuration** (D-11).
    // Works indicate a theme better than authors, and authors better than tags —
    // and that ordering is not a preference the operator may have, because it is
    // what makes the theme mean anything.
    for (tag, body) in [
        (
            "inverted",
            "[theme.signal_weights]\nworks = 0.1\nauthors = 0.3\ntags = 0.6\n",
        ),
        (
            "authors-above-works",
            "[theme.signal_weights]\nworks = 0.2\nauthors = 0.7\ntags = 0.1\n",
        ),
        (
            "equal",
            "[theme.signal_weights]\nworks = 0.4\nauthors = 0.4\ntags = 0.2\n",
        ),
    ] {
        let error = load(tag, body).expect_err("an inverted ordering must be refused");
        assert!(
            error.to_string().contains("theme.signal_weights"),
            "{tag}: the refusal names the key: {error}"
        );
    }
}

#[test]
fn a_guest_may_never_earn_trust() {
    // **False, always** (spec §14.1): a guest can never extend trust. A
    // configuration that says otherwise is refused rather than ignored, because an
    // ignored setting that grants standing is the worst kind.
    let error = load("guest-earns", "[trust]\nguest_earns = true\n")
        .expect_err("trust.guest_earns = true must be refused");
    assert!(
        error.to_string().contains("trust.guest_earns"),
        "the refusal names the key: {error}"
    );
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
