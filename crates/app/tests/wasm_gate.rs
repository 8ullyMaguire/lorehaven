//! M45-23 step 9 — §55.6's gate, as a test.
//!
//! Spec §55 specifies both the declarative and the WASM adapter paths, and ships
//! only the first. `wasmi` is named in §21.4 as the intended runtime and is
//! deliberately not a dependency of this workspace: Path B's entire security
//! story rests on a domain lockdown (§55.4.1) whose implementation does not
//! exist, and shipping the path before the lockdown has been written and attacked
//! would make every other §21 guarantee conditional on one unexamined function.
//!
//! **This test is the gate.** It asserts on the manifests themselves rather than
//! on this file's existence, because a test asserting "a file that documents
//! the gate still exists" stops meaning anything the moment someone deletes it —
//! which is exactly what a well-meaning implementer of step 10 would do.
//!
//! **It has been seen red.** Adding `wasmi = "0.4"` to `crates/app/Cargo.toml`
//! makes this fail, which is the only reason to believe it would catch anything.

/// The manifests that would carry a WASM runtime if Path B were adopted.
///
/// Listed explicitly rather than globbing every `Cargo.toml` in the tree: a glob
/// would also read `target/` and the vendored registry, and a gate that is slow
/// or noisy gets ignored, which is how a gate stops being a gate.
const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "crates/app/Cargo.toml",
    "crates/domain/Cargo.toml",
    "crates/scrapers/Cargo.toml",
    "crates/db/Cargo.toml",
];

/// Resolve a workspace-relative path from the crate directory this test runs in.
///
/// `cargo test -p lorehaven-app` runs with the crate directory as the working
/// directory, so `crates/app/Cargo.toml` is `../Cargo.toml` from here. Getting
/// this wrong produces a test that reads nothing and passes — so the existence of
/// each file is asserted rather than assumed.
fn workspace_manifest(rel: &str) -> std::path::PathBuf {
    let direct = std::path::Path::new(rel);
    if direct.exists() {
        return direct.to_path_buf();
    }
    // One level up: crates/<name>/ -> crates/ -> repo root.
    let from_crate = std::path::Path::new("..").join("..").join(rel);
    assert!(
        from_crate.exists(),
        "{rel} not found from {} or from the crate's parent — this test would read \
         nothing and pass, which is worse than failing",
        std::env::current_dir().unwrap_or_default().display()
    );
    from_crate
}

#[test]
fn no_wasm_runtime_is_adopted_while_the_sandbox_is_unwritten() {
    for rel in MANIFESTS {
        let path = workspace_manifest(rel);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        for needle in ["wasmi", "wasmtime", "wasm-bindgen"] {
            assert!(
                !text.contains(needle),
                "{} adopts `{needle}`, but §55.6's gate is unmet: the sandbox does not \
                 exist and §55.4.1's lockdown has not been written and attacked. Either \
                 build the sandbox and the domain-lockdown test first, or amend §55.6.",
                path.display()
            );
        }
    }
}

/// §21.4 names `wasmi` as the intended runtime, so the gate has to be a decision
/// about this workspace rather than an accident of it. This asserts the decision
/// is still recorded where someone implementing Path B will read it.
#[test]
fn the_spec_still_records_the_gate_and_its_reason() {
    // Through the same resolver as the manifests: `cargo test -p lorehaven-app`
    // runs with the crate directory as cwd, so the repo root is two levels up.
    let spec = workspace_manifest("docs/spec.md");
    let text = std::fs::read_to_string(&spec).expect("read spec");

    assert!(
        text.contains("### 55.6 WASM is gated"),
        "§55.6 has been renamed or removed. The gate is the reason no WASM runtime is \
         adopted, and losing the section loses the reasoning."
    );

    // The three conditions, not just the heading: a gate that lists one condition
    // is a gate with a hole in it. Matched against whitespace-normalised text
    // because the spec hard-wraps, and a phrase broken across a line break is
    // still present.
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for phrase in [
        "the sandbox exists",
        "has a test that proves a private address is refused",
        "a second reviewer has read it",
    ] {
        assert!(
            flat.contains(phrase),
            "§55.6 no longer says `{phrase}` — one of the gate's conditions has gone, so \
             the gate is weaker than it reads"
        );
    }
}

/// Path A is what ships, so it must actually exist in this workspace. A gate test
/// that passes because nothing was built at all would be a third way to be wrong.
#[test]
fn the_declarative_path_is_real() {
    let schema = workspace_manifest("crates/scrapers/src/source_manifest.rs");
    assert!(
        schema.exists(),
        "the declarative manifest schema is missing — §55.6 gates the WASM path on \
         the declarative path being in production, and there is nothing to gate on"
    );
    let adapter = workspace_manifest("crates/scrapers/src/declarative.rs");
    assert!(adapter.exists(), "DeclarativeAdapter is missing");
}
