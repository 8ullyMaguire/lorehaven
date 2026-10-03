# M45-23 — Curator-submitted source adapters

Spec: `docs/spec.md` §55. Plan. Tracker row: `docs/requirements.csv` `M45-23`.

**Read this first.** §55 has two paths. This plan builds **Path A (declarative)**
completely and **specifies Path B (WASM) without implementing it**, because §55.6
gates it on a sandbox that does not exist. Steps 8–10 make that gate checkable
rather than aspirational. If you are implementing and find yourself adding
`wasmi` to a `Cargo.toml` before step 9 is green, stop — that is the failure this
plan is shaped to prevent.

## What already exists (do not rebuild)

| Thing | Where | Note |
|---|---|---|
| SSRF refusal | `crates/scrapers/src/safety.rs` `validate_url` (L1618), `is_forbidden_ip` (L1755) | Already rejects loopback, private, link-local incl. `169.254.169.254`, and `::ffff:` mapped forms. **Tested** at L2750. |
| `SafeFetcher` | `safety.rs` `SafeFetcher::new(allowed_hosts, policy)` | The domain lockdown already exists as a constructor parameter. |
| robots + pacing | `safety.rs` `robots_gate`, `robots.rs` | `Crawl-delay`, 1/s floor, `Disallow`, operator override. |
| `FixtureFetcher` | `safety.rs` L1532 | Offline parsing harness. |
| `SourceAdapter` trait | `crates/scrapers/src/registry.rs` | `route(url)`, `by_key`, `catalogue`, `disable`. |
| 11 adapters | `crates/scrapers/src/sites/` | ao3, chyoa, efiction, ffnet, ficbook, pawchive, royalroad, scribblehub, syosetu, wattpad, xenforo |
| 65 HTML fixtures | `crates/scrapers/tests/fixtures/` | §11.7's discipline is already paid for. |
| Extension domain | `crates/domain/src/extension.rs` | `ExtensionState`, `Capability`, `MemoryTier`, `grant_is_subset`. |

**What does not exist:** `Manifest` struct, `Category` enum, any YAML parser
(`serde_yaml` is not a dependency), and network/credential capabilities.

---

## Step 1 — Turn §21.1's prose manifest into types

**File:** `crates/domain/src/extension.rs`

Add `Category` (from §21.2's list, with `source_adapters` new), and a
`Manifest` struct with §21.1's eleven fields. Add two `Capability` variants:
`Network` and `CredentialRead`, because §55 needs both and `grant_is_subset`
must be able to reason about them.

```rust
/// §21.2's category list, now a type. `SourceAdapters` is new in §55.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ReaderWidget, DashboardWidget, ThemeLayout, RecommendationEngine,
    DeclarativeRecipe, SearchHelper, WritingTool, ChallengeVariant,
    PositivityFilterRule, MoodTag, FandomLanding, Integration,
    /// §55. Curator-submitted source adapters.
    SourceAdapters,
}
```

`Manifest` needs `#[serde(deny_unknown_fields)]`. A typo'd key in a curator's
manifest must be a parse error, not a silently-defaulted field — the whole review
argument for Path A is that a steward reads the manifest and sees what it does.

**Verify**

```sh
cargo test -p lorehaven-domain --lib extension 2>&1 | grep 'test result'
```

- `every_category_round_trips_through_its_slug`
- `an_unknown_manifest_field_is_refused` — proves `deny_unknown_fields` bites.
  Break it by removing the attribute and confirming red.
- `a_grant_cannot_exceed_a_manifest_asking_for_network` — `grant_is_subset` with
  `Network`.

---

## Step 2 — The declarative manifest schema

**File:** `crates/domain/src/source_adapter.rs` (new)

```rust
/// A §55.3 declarative source adapter: URLs and selectors, no code.
///
/// Every selector is compiled at construction. A manifest that would throw on
/// first fetch is refused at submission, where a steward can read the reason,
/// rather than discovered by a reader's import.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub source_id: String,
    pub name: String,
    /// Scheme + host, no trailing path. The allowlist is exactly this host and
    /// its subdomains — §55.4.1.
    pub base_url: String,
    /// Minimum seconds between requests. The host may lower this and never
    /// raises it; §11.5's `Crawl-delay` and the 1/s floor win on conflict.
    pub rate_limit_per_second: f64,
    pub work_pattern: String,
    pub chapter_pattern: String,
    pub selectors: Selectors,
    pub pagination: Pagination,
    pub auth: Auth,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selectors {
    pub title: String,
    pub author: String,
    pub summary: String,
    pub body: String,
    pub tags: String,
    pub word_count: String,
    pub date_published: String,
}
```

Plus `Pagination` (`next_link { selector }` | `none`), `Auth`
(`none | cookie_login { login_url } | api_key | oauth`), and:

```rust
impl SourceManifest {
    /// Every selector compiles, and the base URL passes §11.5's validator.
    /// Returns every problem at once — a reviewer fixing one selector at a time
    /// across three round-trips is a worse review than a list.
    pub fn validate(&self) -> Result<CompiledSource, Vec<String>>;
}
```

**`CompiledSource` holds the `scraper::Selector` values**, so the hot path never
re-parses a selector string, and §55.3's "compiles at submission" is a type-level
property rather than a promise.

**Verify**

```sh
cargo test -p lorehaven-domain --lib source_adapter 2>&1 | grep 'test result'
```

- `a_manifest_whose_selector_does_not_compile_is_refused_naming_it`
- `a_base_url_pointing_at_a_private_address_is_refused` — `169.254.169.254`,
  `127.0.0.1`, `10.0.0.1`, and the `::ffff:` mapped form. This is §55.8's second
  acceptance line and it must fail at **submission**, before any fetch.
- `an_unknown_selector_key_is_refused`
- `a_manifest_reports_every_problem_not_just_the_first`
- `a_valid_manifest_compiles_once_and_caches_its_selectors`

**Note on the private-address test:** assert on the *rule*
(`validate_url` refuses it), not on a hand-listed set of literals — otherwise the
test passes while `172.16.0.1` slips through. `safety.rs` L1760 already has the
complete predicate; call it.

---

## Step 3 — Load YAML

**File:** `crates/domain/Cargo.toml`, `crates/domain/src/source_adapter.rs`

`serde_yaml` is **not** currently a dependency. Before adding it, note the
decision in the commit: it is the standard, it is what makes a curator's
submission reviewable as text, and an alternative (JSON-only) would make §55.3's
"a steward can read this" argument weaker.

Manifests are read from a `source.yaml` inside the submission, per §21.1's
`entrypoint`.

**Verify**

```sh
cargo test -p lorehaven-domain --lib source_adapter 2>&1 | grep 'test result'
```

- `a_yaml_manifest_parses_into_the_same_value_as_its_json` — same structure,
  both literals in the test, so the two encodings cannot drift.
- `the_specs_own_example_manifest_parses` — embed §55.3's YAML verbatim as a
  `const` and parse it. **This test is what keeps the spec's example real**: if
  the schema changes, the spec's example stops compiling and someone has to
  decide whether the spec or the schema is wrong.

---

## Step 4 — Implement `SourceAdapter` for it

**File:** `crates/scrapers/src/declarative.rs` (new)

```rust
/// Adapts a §55.3 `SourceManifest` to the `SourceAdapter` trait.
///
/// It receives a `Fetcher` for the call and holds none, exactly like the
/// compiled adapters — §11.1's structural rule, and the reason every guard in
/// §11.5 is inherited rather than reimplemented.
pub struct DeclarativeAdapter { compiled: CompiledSource }
```

`identify` matches `work_pattern`; `fetch_metadata` and `fetch_chapter` go
through the handed-in `Fetcher` and the compiled selectors.

**Verify**

```sh
cargo test -p lorehaven-scrapers --lib declarative 2>&1 | grep 'test result'
```

The load-bearing test is parity, because §55.8's fourth acceptance line demands it:

```rust
#[tokio::test]
async fn a_declarative_adapter_and_the_handwritten_one_agree() {
    // Same fixture, both paths, same work.
    let compiled = CompiledSource::compile(&manifest()).unwrap();
    let declarative = DeclarativeAdapter::new(compiled);
    let handwritten = Ao3Adapter::new();

    let url = Url::parse("https://archiveofourown.org/works/12345").unwrap();
    let a = declarative.preview_from_html(&fixture("ao3/work-full.html"), &url).unwrap();
    let b = handwritten.preview_from_html(&fixture("ao3/work-full.html"), &url).unwrap();
    assert_eq!(a, b, "a steward must be able to trust the manifest means the adapter");
}
```

The real entry points are `preview_from_html` and `chapters_from_html` (AO3
L546), taken from the `SourceAdapter` trait — they are the offline half of
§11.1's "handed in, never held" rule, and they are why this test needs no fetcher.

This runs offline via `FixtureFetcher` — no network, deterministic.

**Also verify:** the adapter cannot fetch a URL outside its `base_url`. Break the
allowlist and confirm the test goes red; this is §55.4.1 and it is the one
property Path B has no equivalent of.

---

## Step 5 — Migration 0113: submissions and reviews

**Files:** `migrations/sqlite/0113_source_adapters.sql`,
`migrations/postgres/0113_source_adapters.sql`

Two tables. `extension_submissions` is the §55.2 pipeline's state
(`pending | approved | rejected | revoked`); `adapter_reviews` is §19.4's
three-reviewer record, one row per reviewer, and the UNIQUE
`(submission_id, reviewer_account_id)` is what makes "three reviewers" a
constraint rather than a hope.

PostgreSQL: `id UUID PRIMARY KEY`, `created_at TIMESTAMPTZ`, `manifest JSONB`.
Follow `0112`'s comment style for why each convention is what it is.

**Verify**

```sh
cargo test -p lorehaven-db --lib migration 2>&1 | grep -E '0113|test result'
cargo run --bin lorehaven -- migrate 2>&1 | tail -3
```

Expect **113/113** on both dialects. Then query both tables on both engines —
a PostgreSQL `TIMESTAMPTZ`/`JSONB` mismatch surfaces only when you `SELECT`.

---

## Step 6 — Store, scoped and trusted

**File:** `crates/db/src/source_adapters.rs` (new)

- `submit(db, account, manifest, trust_level) -> Result<String>` — **refuses
  below TL3**, naming the bar (§55.2). This is the check that matters; it must be
  in the store, not only in the route, because the route is not the only caller.
- `list_pending(db)`
- `record_review(db, submission_id, reviewer, verdict)` — the UNIQUE makes a
  second review by the same reviewer an error, not a second row.
- `published_manifests(db) -> Vec<SourceManifest>` — what the registry loads.

**Verify**

```sh
cargo test -p lorehaven-db --lib source_adapters 2>&1 | grep 'test result'
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-db --lib source_adapters 2>&1 | grep 'test result'
```

- `a_curator_below_tl3_is_refused` — delete the trust check, confirm red.
  **This is the single most important test in the plan.**
- `the_same_reviewer_cannot_review_twice`
- `a_submission_becomes_published_only_at_three_approvals` (§19.4)

---

## Step 7 — Routes

**File:** `crates/app/src/routes/source_adapters.rs` (new), merged in
`crates/app/src/server.rs`

`POST /api/v1/extensions/source-adapters` (submit), `GET .../{id}/reviews`,
`POST .../{id}/reviews`. Every one must be in `ROUTE_TABLE` or
`registered_routes_are_tabled` fails — and it stops at the *first* missing one,
so search the table for `source_adapters.rs` directly.

**Verify:** `cargo test -p lorehaven-app --test route_inventory` and a new
`source_adapter_routes.rs` covering §55.8's 401/403 lines.

---

## Step 8 — The automated check a reviewer reads

**File:** `crates/scrapers/src/declarative_check.rs` (new)

§55.5's gate: fetch three known works, validate against §4.3's shapes, verify
robots and pacing, and produce a report with sample parses.

Run against `FixtureFetcher` by default and the live fetcher when the operator
opts in. The report is a value a reviewer reads; §55.5 is worthless without it.

**Verify:** `cargo test -p lorehaven-scrapers --lib declarative_check`, including
`a_check_report_names_the_manifest_version_it_checked` — a report that does not
name its version is a report about nothing.

---

## Step 9 — The §55.6 gate, as a test

This is what makes "Path B is not built yet" a fact rather than a promise.

**File:** `crates/app/tests/wasm_gate.rs` (new)

```rust
/// §55.6: no `wasmi` dependency until the sandbox exists and the domain lockdown
/// has been attacked.
///
/// This test is the gate. Delete it and nothing stops step 10 from happening, so
/// it asserts on the manifests themselves rather than on this file's existence.
#[test]
fn no_wasm_runtime_is_adopted_while_the_sandbox_is_unwritten() {
    for manifest in ["Cargo.toml", "crates/app/Cargo.toml", "crates/domain/Cargo.toml"] {
        let text = std::fs::read_to_string(manifest).expect("read manifest");
        assert!(
            !text.contains("wasmi") && !text.contains("wasmtime"),
            "{manifest} adopts a WASM runtime, but §55.6's gate is unmet: the sandbox \
             does not exist and §55.4.1's lockdown is untested"
        );
    }
}
```

**Verify it bites** — this is the step people skip:

```sh
cargo test -p lorehaven-app --test wasm_gate 2>&1 | grep 'test result'
# then: add `wasmi = "0.4"` to crates/app/Cargo.toml, re-run, confirm RED, revert.
```

An `assert!(!contains("wasmi"))` that has never been seen red is the same class
of non-evidence as a green test on unfixed code.

---

## Step 10 — Tracker, docs, definition of done

`docs/requirements.csv` is **CRLF**. Do not rewrite it with Python's `csv`
module — that converts every line ending and turns a 1-line edit into 698. Split
on `"\r\n"`, edit the one line, rejoin with `"\r\n"`.

Mark M45-23 as covering **Path A**, and record explicitly that Path B is
specified and gated. Do not mark it "done" as though both paths shipped.

Also update §21.2's category list to name `source_adapters`, and add a line to
`docs/plans/WHAT-IS-LEFT.md`.

**Definition of done**

- [ ] §55.8's acceptance criteria have named tests, green on **both** engines.
- [ ] The spec's own example manifest parses (step 3).
- [ ] Declarative and handwritten adapters agree on a real fixture (step 4).
- [ ] `cargo clippy --workspace --all-targets` 0 warnings.
- [ ] Every new test seen red.
- [ ] `wasm_gate` green **and** seen red.
- [ ] 113/113 migrations on both dialects.
- [ ] Tracker says "Path A shipped, Path B gated" — not "done".

## Traps

- **`serde_yaml` is a new dependency.** Justify it in the commit (§55.3's
  reviewability argument depends on it).
- **Do not add a browser-driving host call.** §55.1 declines it: it would put a
  paid third-party service inside the sandbox and cross §11.5's
  anti-circumvention line. `scribblehub` and `ffnet` stay solver-gated.
- **`is_forbidden_ip` is the rule, not a literal list.** Assert on the predicate.
- **Test binaries using relative paths run from their own crate directory.**
- **`git diff` is intercepted in this repo** and can report "No syntactic changes"
  for a file that genuinely differs — use `diff <(git show HEAD:f) f`.
- **Never `git add -A` a directory while a test binary is running.**
