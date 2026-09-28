# Crawling posture, retention governance, and preservation — implementation plan

**Date:** 2026-09-27
**Spec:** `docs/spec-amendments/crawling-retention-and-preservation.md`
**Milestone:** M59
**Status:** Phases A0 and A.1–A.3 built and green. Phase A.4, B, C1, C, D, E
not started.

Read the amendment before this file. This file is the build order, the file
paths, and the verification commands. Every phase below names the amendment
section it implements, because the amendment holds the reasoning and this file
holds the typing.

**Migration numbers are 0085 (A0), 0086 (C), 0087 (D), 0088 (E).** They were
written as 0084–0087, and shifted twice — once when A0 was inserted, and once
when A0 was built and found 0084 already reserved by migration 0083. See the
note at the end of this file.

---

## 0. What exists, and what this is actually building

| Piece | Spec status | Code status | This plan |
|---|---|---|---|
| `honour_robots` override | live, §11.5 | **built** — `FetchPolicy::honour_robots`, `robots_gate`, per-host counter, first-per-host log | Phase A amends it — **done, A.2** |
| `robots_gate` as a pure function | §11.5 | built, `safety.rs:1316` | Phase A extends the signature — **done, A.3** |
| `cache \| aggregate` retention | §11.15, fully specified | **not built** — M6-15 `unsupported`: no setting, no refusal path, no admin route | Phase C |
| Crossposting | §11.12 | not built (no `crosspost` symbol outside one test file name) | Phase D |
| Preservation batches | §11.11 | not built — M6-10 `unsupported` | out of scope; Phase D reuses the destination machinery only |
| Retention voting | new §11.15a | n/a | Phase E |
| Ledger / credits | §20.1 | built — `db::economy::post_transaction` with `idempotency_key` | Phase D reuses it |
| Jobs | §10.4 | built — `job_kinds!` emits the enum, `ALL_KINDS`, `as_str`, `parse` from one tree | Phases D, E add kinds |
| Trust ladder | §19.1 | built — `db::governance::trust_for` | all gated phases |

**Two prerequisites that are not obvious and that Phase E depends on.**

1. **§29.4's "configured minimum trust level" is not configured.**
   `crates/app/src/routes/roadmap.rs:80`, `:136` and `:250` all read
   `if trust < 1`. The spec says "the configured minimum trust level", so the
   spec and the code disagree. The amendment's §19.15 leans on §29.4 as
   *precedent for configurability*, and the precedent is a claim, not a
   working feature. **Either wire `roadmap.min_trust` as part of Phase E, or do
   not cite §29.4** — an amendment that justifies a configurable bar by pointing
   at a hardcoded one is exactly the kind of citation that rots. Prefer wiring
   it: it is four lines and it makes the citation true.
2. **`requirements.csv` has no status value for "specified, not built".** The
   four in use are `implemented-fully-tested`, `implemented-locally-tested`,
   `implemented-verified-e2e`, `planned`, `unsupported`. Phase A/B rows are
   amendments to a *working* mechanism, so they are `planned` until built.
   §11.15a/b and §19.15 rows are new sections with no code, which is what
   `planned` means. **No new status value is introduced** — `planned` is
   honest for all of them and adding a value would be a vocabulary change the
   owner did not ask for.

---

## Phase A0 — Cross-source identity, the minimum Phase D needs

Amendment §11.10b. **New on 2026-09-27**, after a probe found that §11.10's four
tables have never existed. This phase carries the first migration in the plan
and is the prerequisite for Phase D.

### A0.1 Migration 0085, both dialects

`migrations/sqlite/0085_story_identity.sql` and
`migrations/postgres/0085_story_identity.sql`, identical ids:

```sql
CREATE TABLE IF NOT EXISTS story_identities (
    id                TEXT PRIMARY KEY,
    -- The local work this identity is about. A crosspost this instance
    -- performed always has one; an identity with no local member is a
    -- purely-external grouping, which A0 does not create.
    work_id           TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- Canonical metadata is a COPY of the work's, not the authority. §11.10's
    -- merges are reversible, so an identity must be able to be dissolved back
    -- into its members without a work losing its title.
    canonical_title   TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'active',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS story_identity_members (
    id                  TEXT PRIMARY KEY,
    identity_id         TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    -- Exactly one of the two. A local member names a work; an external member
    -- names a record at a site. Both are CHECK-enforced, because a row with
    -- both or neither is a row that means two things.
    work_id             TEXT REFERENCES works (id) ON DELETE CASCADE,
    external_record_id  TEXT,
    -- One value ships: 'cross_posted'. See below before adding a second.
    edition_relation    TEXT NOT NULL DEFAULT 'cross_posted',
    external_source_key TEXT,
    external_url        TEXT,
    created_at          TEXT NOT NULL,
    UNIQUE (identity_id, work_id),
    UNIQUE (identity_id, external_source_key, external_url),
    CHECK ((work_id IS NULL) <> (external_record_id IS NULL))
);
```

**Both tables from §3 are created empty** — `identity_merge_proposals` and
`identity_merge_history` — so the data model is honest and
`docs/requirements.csv` can seed a row for each. Nothing writes to them in A0.

Retention behaviour, per §4.1: a member cascades with its identity and with its
work. A member is **not** deleted when its external site dies — it is marked
unavailable, because a vanished edition is a fact about the world and deleting
the row would lose the provenance. Merges, when they exist, go in
`identity_merge_history`, never as a row deletion.

### A0.2 One relation, because one is what this instance can know

`edition_relation` ships with exactly one value: `cross_posted`. That is not
timidity, it is the only relation A0 can establish **without guessing**, because
the instance performed the act itself.

**No inference path may be written.** Not from title+author similarity, not from
canonical URL, not from a shared tag set. A `translation` recorded as
`cross_posted` tells a reader two texts are one; an `unrelated_lookalike`
recorded as `cross_posted` is the specific failure §11.10 exists to prevent. The
test to write:

```text
an_identity_member_cannot_be_created_by_similarity_of_title_author_or_url
```

A grep for the matcher is a second net, but the test is the one that matters:
assert that every member row's `created_by` is a performed crosspost.

### A0.3 What the reader sees

A work page lists the identity's members as editions: this instance's copy
first, then each external location with its own link. **"Do not grant access to
another edition's body"** (§11.10) is the rule that matters here — a member row
records *that* a copy exists and *where*, never a body, and a reader with this
instance's copy gains nothing from a member row.

Verification state (`verified | unverified | dead`) is Phase D's column on this
row, added in 0087. A0 leaves it `unverified` for every external member, which
is honest: nothing has checked.

### A0.4 Verification

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-app story_identity 2>&1 | tail -30
```

```text
a_work_can_hold_an_external_member_naming_a_location_with_relation_cross_posted
a_member_row_holds_exactly_one_of_work_id_or_external_record_id
an_identity_resolves_to_one_work_page_listing_local_and_external_copies
an_external_member_grants_no_access_to_any_body
a_member_is_not_deleted_when_its_external_site_is_unavailable
the_two_merge_tables_exist_and_nothing_writes_to_them
an_identity_member_cannot_be_created_by_similarity_of_title_author_or_url
every_member_row_names_a_crosspost_this_instance_performed
the_two_dialects_define_the_same_migration_ids
```

The last-but-one and the parity test are the two that catch the failure modes
that matter. A member created by a guess is unrecoverable — it tells a reader
two different texts are the same book, and no later fix removes the wrong
linkage that a reader already believed.

---

## Phase A — `robots_posture` and the fetch class

Amendment §1. Smallest phase, unblocks everything else, and the only phase
that touches a security boundary.

### A.1 `FetchClass` in the domain

New file `crates/scrapers/src/robots.rs` addition (the rules live there; the
class is a robots concept because only robots differentiates the two):

```rust
/// What kind of read this is (spec §11.5, `FetchClass`).
///
/// Set at the call site by the adapter that declares it, never inferred from
/// a URL. A chapter-shaped URL fetched as `Metadata` is still bounded as a
/// metadata fetch, because inference from shape is how a metadata door
/// becomes a body door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchClass {
    /// Title, author, tags, chapter list. Bounded at 1 MiB and parsed into a
    /// type with no body field.
    Metadata,
    /// Prose. The default for anything an adapter calls a chapter.
    Content,
    /// Image, audio, video bytes (§30.2's hosting policy decides what happens
    /// to them afterwards).
    Media,
}
```

### A.2 `RobotsPosture` replaces the boolean

`crates/app/src/config.rs` (`ImportsConfig`, around line 1104):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RobotsPosture {
    /// Honour `Disallow` for every class. The default, and today's
    /// `honour_robots = true`.
    #[default]
    Strict,
    /// Read a disallowed metadata path, store none of it, refuse every
    /// content path outright (spec §11.5).
    MetadataOnly,
    /// Honour nothing. Today's `honour_robots = false`, kept because an
    /// operator sometimes has a reason (§11.5's own justification).
    Permissive,
}
```

`ImportsConfig` gains `pub robots_posture: RobotsPosture` and **keeps**
`pub honour_robots: bool` as a compatibility read. Resolution is one pure
function in `crates/domain/src/policy.rs`:

```rust
/// The posture an instance actually runs, given both keys.
///
/// The posture wins when both are present, so a config carrying a stale
/// `honour_robots = false` from before the migration does not silently
/// override a deliberate posture. (`false` -> `Permissive`, `true` ->
/// `Strict`.)
pub fn resolve_posture(posture: RobotsPosture, honour_robots: Option<bool>) -> RobotsPosture
```

**Built as two deviations from the sketch above.** Both are recorded here
because the sketch is the thing a later reader trusts, and a divergence that
is not written down is indistinguishable from a mistake.

1. **`Option<RobotsPosture>`, not `RobotsPosture`.** The signature above cannot
   distinguish an operator's deliberate `strict` from the same word arriving by
   default, so with the non-optional type the default `Strict` beat
   `honour_robots = false` on *every* existing config: the key parsed, the file
   was accepted, and nothing obeyed it — worse than not having the key, because
   the operator's own words are in the file. `None` means "named nothing" and is
   what makes the compatibility fallback reachable. Mutation M3/M4 in the proof
   below covers both directions.

2. **`resolve_posture` lives in `crates/scrapers/src/robots.rs`, not
   `crates/domain/src/policy.rs`.** `scrapers` does not depend on `domain`, and
   the two are siblings over `lore-metadata`, so the plan's location would have
   meant a new crate edge for a function that resolves a robots concept.
   `app::config::ImportsConfig::resolved_robots_posture` is the only caller, so
   the one-hop indirection buys nothing and the edge costs a build-order
   constraint. Reached through `app` either way.

The gate itself reads `FetchPolicy::robots_posture`; `honour_robots` is
re-derived from the resolved posture inside `policy_for` so a caller reading the
deprecated field is never told the opposite of the truth. That re-derivation was
wrong in the first version — it stored `matches!(posture, Permissive)`, the
answer to its own question rather than to the one the field means — and the
pre-existing `the_import_policy_carries_the_instances_robots_answer` test caught
it. Kept as mutation M8, which reproduces exactly that inversion.

An **unrecognised** posture string stops startup, matching how
`access_mode` and `rec.mode` already behave. A typo that fell back to
`Strict` would look like compliance while crawling `Permissive`-intent.

### A.3 `robots_gate` takes the class

`crates/scrapers/src/safety.rs:1316`, currently
`fn robots_gate(rules, path, honour_robots: bool) -> RobotsGate`:

```rust
pub enum RobotsGate {
    Allowed,
    Refused,
    /// Posture is `MetadataOnly` and the rules forbid this path: read it,
    /// keep nothing. Distinct from `Allowed` because the bytes are discarded,
    /// and from `Overridden` because nothing was overridden.
    ReadAndDiscarded,
    Overridden,
}
```

The decision, as a pure function of `(rules, path, posture, class)`:

| `rules.allows(path)` | posture | class | result |
|---|---|---|---|
| true | any | any | `Allowed` |
| false | `Strict` | any | `Refused` |
| false | `MetadataOnly` | `Metadata` | `ReadAndDiscarded` |
| false | `MetadataOnly` | `Content` \| `Media` | `Refused` |
| false | `Permissive` | any | `Overridden` |

Pacing is untouched by every row. `Crawl-delay` and the one-second floor
(`default_interval_per_host`) are read and enforced identically under all
three postures — **asserted by a test, not by a comment**, because that is the
one property most likely to be broken by a later edit.

### A.1–A.3 verification

Twelve mutations, each of which must turn the suite red. Every one is a way the
change can be wrong while still compiling and still looking correct:

| # | mutation | caught by |
|---|---|---|
| M1 | `metadata_only` lets `Content` through | `metadata_only_still_refuses_every_content_and_media_path` |
| M2 | an allowed path is gated on posture | `an_allowed_path_is_allowed_under_every_posture_and_class` |
| M3 | the stale bool outvotes the posture | `the_posture_wins_over_a_stale_honour_robots` |
| M4 | the compatibility key is ignored entirely | `honour_robots_still_selects_permissive_when_no_posture_is_named` |
| M5 | an unrecognised posture defaults to `Strict` | `an_unrecognised_posture_stops_startup_rather_than_defaulting` |
| M6 | an undeclared fetch defaults to `Metadata` | `an_undeclared_fetch_is_refused_under_metadata_only` |
| M7 | `policy_for` never sets the posture | `the_import_policy_carries_the_resolved_robots_posture` |
| M8 | `honour_robots` re-derived inverted | `the_import_policy_carries_the_instances_robots_answer` |
| M9 | a declared metadata fetch is refused | `a_declared_metadata_fetch_under_metadata_only_reaches_the_network` |
| M10 | a read-and-discard is counted as an override | `a_read_and_discarded_fetch_is_not_counted_as_an_override` |
| M11 | `ReadAndDiscarded` becomes `Overridden` | `the_four_answers_are_the_whole_decision` |
| M12 | the gate ignores the class | `metadata_only_reads_a_disallowed_metadata_path_and_discards_it` |
| M13 | `ReadAndDiscarded` becomes a refusal | `a_declared_metadata_fetch_under_metadata_only_reaches_the_network` |
| M14 | the catalogue note always names `robots_posture` | `an_instance_that_overrides_disallow_says_so_in_the_catalogue` |
| M15 | the catalogue note always names `honour_robots` | `the_catalogue_names_the_posture_when_the_posture_is_what_decided` |
| M16 | `robots_posture` dropped from the catalogue wire | `the_catalogue_names_the_posture_when_the_posture_is_what_decided` |

M14 is a real bug this phase shipped, caught by a test written before the change
rather than after it. The note unconditionally named
`imports.robots_posture = permissive`, which is wrong for **every existing
config** — none of them name a posture, so all of them reach `permissive` through
`honour_robots = false`. The operator would have been pointed at a key absent
from their file, where setting it is a no-op because the value already *is*
`permissive`. An error message that names the wrong key is worse than none,
because the reader acts on it. The note now branches on which key decided the
answer, and M14/M15 pin both directions.

M6, M7 and M10 **survived the first pass** and each found a real hole rather than
a bad assertion. M6 and M7 passed only because no test read the field the gate
reads — the existing test asserted the *deprecated* bool, which `policy_for`
happens to keep in step. M10 passed because the counter is not part of what the
gate returns, so routing the discarded read through `record_robots_override`
satisfied every gate assertion. The three tests that closed them are named in
the table.

Two of the first-pass runs were also untrustworthy and are worth recording: M3
and M4 reported through the wrong crate, and M10's substitution did not match
the source (the arm carries a five-line comment, so the bare arm text is not
present) and silently changed nothing. A mutation that fails to apply reads
exactly like one that survived. Both scripts now assert the anchor count and
print `mutation applied` before running, and the runners report a compile error
as distinct from a pass.

**A.1–A.3 are done. A.4 is not, and it is the half that makes `ReadAndDiscarded`
mean anything.** The gate now answers `ReadAndDiscarded` and the fetcher now acts
on it, but nothing yet stops a caller that received a discarded body from
storing it — the arm's doc comment says a later phase enforces that, and that
comment is currently the only thing between a discarded body and the database.
A.4 is where the 1 MiB ceiling and the body-less parse target go.

### A.4 The metadata fetch cannot return a body

Three limits, all required (amendment §1.2):

1. `FetchPolicy::max_bytes_for(class)` — `Metadata` = 1 MiB,
   `Content`/`Media` = the existing 8 MiB. Exceeding it is a **truncation that
   is recorded on the job report**, not a silent short read.
2. The metadata parse target is a type with no body field. If the adapters
   already parse into something with a `body: String`, that field moves behind
   the `Content` path — check `crates/scrapers/src/sites/*.rs` for
   `preview_from_html` / `chapters_from_html` and confirm the metadata struct
   has no prose field before assuming it does.
3. A `Metadata` fetch writes metadata rows only: no `db::content::append_revision`,
   no FTS row, no cache-fill enqueue. This is the limit that actually holds
   when the other two are wrong.

`policy_for` (`crates/app/src/imports.rs:50`) becomes
`policy_for(adapter, config, class)` and sets
`policy.honour_robots` from `resolve_posture(...)` as it does today, so
**every existing import path picks the posture up at one place**. That is the
property the whole amendment depends on: an adapter that could opt out of the
posture would make the rule advisory.

### A.5 Per-source, run-scoped overrides

`[imports] robots_posture_overrides` is a map of source family → posture,
**narrowing only**. A widening entry is refused at config load with a named
error, matching the §11.15 source-override rule that an override may only
narrow.

Expiry is not a timer: the override is consulted through an
`OverrideScope` handed to the import runner and dropped when the run ends, so
there is no clock and nothing to clean up. A process that outlives one run
holds the override in the same `RunScope` value the robots TTL already uses.

### A.6 Verification

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-scrapers robots 2>&1 | tail -20
cargo test -p lorehaven-app milestone_6 2>&1 | tail -30
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
```

Expected: the existing `a_disallowed_path_is_refused_before_any_request_is_made`
and `a_disallowed_path_is_refused_when_the_instance_honours_the_rules` still
pass **unmodified** — they are the regression net for the `Strict` path. New
tests, named for the properties:

```text
a_metadata_class_on_metadata_only_reads_a_disallowed_path_and_stores_nothing
a_content_class_on_metadata_only_is_refused_naming_the_posture
pacing_is_identical_under_all_three_postures          (same rules, three policies)
a_metadata_fetch_writes_no_revision_and_no_fts_row
a_refused_content_fetch_is_not_retried
a_per_source_override_may_only_narrow
an_unrecognised_posture_stops_startup
honour_robots_false_alone_resolves_to_permissive
both_keys_present_resolves_to_the_posture
```

**Both dialects.** The gate is pure, so this phase has no migration — which is
why it is Phase A and not later.

---

## Phase B — the class-specific User-Agent token, and the impersonation refusal

Amendment §1.5. Small, and it belongs with A because it is the same config
surface.

The token may name the class: `Lorehaven/1.0 (+import; class=metadata)`. It is
appended in `FetchPolicy`'s UA construction, not at the call site, so a
mistake cannot produce a UA that lies about a class it is not making.

**The refusal needs to be in the spec text and in a test, not just here:**
choosing a token to land in a more permissive group is abuse under §24.5.
`crates/scrapers/src/safety.rs` gains a `validate_user_agent(token) -> bool`
that rejects a token naming another product's bot, and a test
`a_token_impersonating_another_crawler_is_refused`. There is no route that
sets a UA, so this is the *only* place a token enters, which is what makes the
check cheap rather than aspirational.

```bash
cargo test -p lorehaven-scrapers user_agent 2>&1 | tail -10
```

---

## Phase C1 — Body audience: who may read a body this instance holds

Amendment §7.7. **New on 2026-09-27**, on the owner's instruction. This is a
narrowing of §11.15's *baseline* (every eligible reader reads a cached body),
not a per-request override — see the amendment's §6.1 for why those are
different decisions.

### C1.1 The change to `can_access_content`

`crates/domain/src/policy.rs:263` already takes an `Actor` and already has a
role-based grant:

```rust
pub struct Actor {
    pub account_id: AccountId,
    pub pseud_id: PseudId,
    pub age_state: AgeState,
    pub trusted_reviewer: bool,     // the only standing field today
}
```

There is **no trust level and no role on `Actor`**, so this is not "add a
check" — it is "give the one function every surface consults a notion of
institutional standing it does not have". Add:

```rust
/// Who this actor is, for a body-audience check (spec §7.7).
/// A grant, never a score: every variant is a fact about a role the account
/// holds, so nothing here can be inferred from a preference signal.
pub struct ActorStanding {
    pub trust_level: i64,        // 0..6
    pub is_operator: bool,
    pub is_vanguard: bool,       // §16.18 membership, NOT a resonance score
    pub is_curator: bool,        // §32 media curator role
}
```

### C1.2 Order of evaluation — the part that must not be got wrong

`policy.rs:281` today returns `Decision::Allow` unconditionally for
`trusted_reviewer`, **before** the lifecycle check and before the rating
ceiling. §7.7.2 says the audience is the opposite and must be placed after:

```text
1. contributor of this work        → allow
2. author blocked the actor        → deny
3. trusted_reviewer                → allow  (EXISTING, unchanged, §7.7.2
                                                does not extend it)
4. lifecycle publicly readable    → deny
5. rating ceiling, age-dependent   → deny   ← runs BEFORE the audience
6. body_audience                   → deny   ← new
```

Step 5 before step 6 is the whole safety property. **A `role:operator`
audience on a `declared_minor` age state must still be refused by the minor
ceiling.** The test that pins it is listed below and it is the one to write
first in this phase.

### C1.3 The audience type and its resolution

New in `crates/domain/src/retention.rs`:

```rust
pub enum BodyAudience {
    Anyone, AccountsOnly, TrustAtLeast(i64), RoleOperator, RoleVanguard, RoleCurator,
}

/// Narrowest wins. All three scopes may only narrow; a widening is refused by
/// name at the setter, not silently ignored.
pub fn resolve_audience(instance: BodyAudience, source: Option<BodyAudience>,
                        work: Option<BodyAudience>) -> BodyAudience
```

Ranking `narrowest` needs an order, and the order is by *how much it removes*:
`RoleVanguard < RoleCurator < RoleOperator < TrustAtLeast(4) < TrustAtLeast(2) <
TrustAtLeast(1) < AccountsOnly < Anyone`. The `TrustAtLeast` arm is
deliberately parameterised rather than one variant per level, so
`TrustAtLeast(4)` is not accidentally equal to `TrustAtLeast(2)`.

`works.body_audience` is **NULL when absent, never the string `'anyone'`** — an
explicit `'anyone'` on a work would override a narrower instance default the
moment anyone edited that work, which is a widening path. `NULL` means inherit.

### C1.4 The indistinguishability work — this is most of the phase

§7.7.3 is the requirement and it touches every surface. The places the current
code would leak, found by reading the spec's own surfaces rather than by
guessing:

```bash
cd ~/code-local/rust/lorehaven
rg -n 'offline|download|export' crates/app/src/routes/library.rs | head
rg -n 'new chapter|chapter.*available' crates/app/src/routes/notifications.rs | head
rg -n 'word_count|chapter_count' crates/app/src/routes/works.rs | head
```

Each hit is a place a body-derived signal can reach a gated reader. The
notification case is the subtle one: "new chapter available" is an existence
oracle for a body, so on a gated work it must say *the work changed* and not
that a chapter arrived.

**The test that matters is a paired-response comparison**, not a set of
assertions about status codes:

```text
a_gated_readers_responses_are_byte_identical_to_a_non_existent_works
```

It normalises the requested id out of both bodies and compares status, headers
and payload across chapter read, work page, search, library, notification,
feed, export and the API. A test that asserts `status == 404` passes while a
`X-Body-Cached: true` header leaks the fact, and that is exactly the shape of
defect this section exists to prevent.

### C1.5 Verification

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-app body_audience 2>&1 | tail -40
cargo test -p lorehaven-domain policy 2>&1 | tail -20
```

```text
the_rating_ceiling_is_evaluated_before_the_body_audience
a_role_operator_audience_on_a_declared_minor_age_state_is_still_refused
a_gated_readers_responses_are_byte_identical_to_a_non_existent_works
a_gated_chapter_request_is_404_with_the_coarse_noun_and_never_403
no_surface_renders_a_disabled_body_affordance_with_a_tooltip
no_route_returns_a_body_audience_a_resonance_or_a_role_membership_list
a_vanguard_gate_consults_membership_and_never_a_resonance_score
a_source_or_work_audience_may_narrow_and_never_widen
a_null_work_audience_inherits_and_an_explicit_anyone_does_not_override_a_narrower_default
on_an_aggregate_instance_the_audience_is_inert_and_every_reader_is_refused_identically
a_trusted_reviewer_branch_is_unchanged_by_this_phase
```

`a_trusted_reviewer_branch_is_unchanged_by_this_phase` is a regression net, not
a feature test: §7.7.2 deliberately leaves the existing unconditional `Allow`
alone, and a future edit that "tidies" it into the audience system would
silently grant operators access above the age ceiling.

---

## Phase C — retention, built as §11.15 specifies

Amendment §4. This is **M6-15**. A0 now carries the first migration, so this
is the second.

### C.1 Migration 0086, both dialects

`migrations/sqlite/0086_instance_retention.sql` and
`migrations/postgres/0086_instance_retention.sql`, identical ids (§2.1 of
`docs/plans/README.md`):

```sql
CREATE TABLE IF NOT EXISTS instance_retention_policy (
    id          TEXT PRIMARY KEY,
    body_mode   TEXT NOT NULL DEFAULT 'cache',   -- cache | aggregate
    updated_by  TEXT REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_instance_retention_policy_singleton
    ON instance_retention_policy (id) WHERE id = 'default';

CREATE TABLE IF NOT EXISTS instance_retention_source_overrides (
    source_key TEXT PRIMARY KEY,
    body_mode  TEXT NOT NULL,                    -- aggregate only; cache may not widen
    updated_by TEXT REFERENCES accounts (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1
);
```

Retention behaviour, stated in the migration comment per §4.1: narrowing to
`aggregate` does **not** delete stored bodies (that is the §10.4 deletion
workflow); widening to `cache` does **not** retro-fetch; both are the
operator's recorded decision and both are in the modlog.

### C.2 The six refusal paths, one enum

New `crates/domain/src/retention.rs`:

```rust
pub enum RetentionReason {
    Aggregate,              // RETENTION_AGGREGATE
    AggregateSourceOverride,// RETENTION_AGGREGATE_SOURCE_OVERRIDE
    SourceBlocked,          // RETENTION_SOURCE_BLOCKED
    Vanished,               // RETENTION_VANISHED
}
```

with `AppError::RetentionDenied { reason, work_id }` carrying a stable code.
Six call sites — URL import, file upload, clipboard paste, preservation batch,
federated announcement, cache fill — all call one function:

```rust
pub fn check_body_allowed(policy: &ResolvedRetention, source_key: Option<&str>)
    -> Result<(), RetentionReason>
```

The reason this is an enum and not six strings: the amendment requires the
refusal paths be *enumerable*, and a test must assert all six exist.

### C.3 Routes and config

`crates/app/src/routes/admin.rs` (or a new `retention.rs` module, whichever
matches how `routes/mod.rs` registers areas):

```text
GET   /api/v1/admin/retention/policy
PATCH /api/v1/admin/retention/policy
GET   /api/v1/admin/retention/sources
PUT   /api/v1/admin/retention/sources/:sourceKey
```

plus the reader-facing door from §6.2:

```text
POST /api/v1/works/:id/body-request
```

Config: `[retention]` with `proposal_mode`, `proposal_min_trust`,
`body_request_min_trust`, `widen_quorum`, `cooling_days`. Note this is a
**new** config section — the existing `docs/spec.md` §38 table has no
`[retention]` block, so the amendment's table is the first statement of it.

### C.4 `works_past_saving`

`crates/db/src/analytics.rs` gains a counted query for aggregated works whose
origin is unreachable and whose body this instance does not hold. It appears
on the operator dashboard only — not on any public surface, and not through
`stats` (the §24.2 public route). "Operator only" is enforced by the analytics
capability registry if the surface is a dashboard, or by route gating if it is
a bespoke admin field; **pick one and pin it with a test** that a non-operator
request 404s.

### C.5 Verification

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-app retention 2>&1 | tail -30
cargo test --workspace 2>&1 | tail -5      # both backends
```

Against live PostgreSQL as well — the `0086` migration must be applied and
tested on both dialects:

```bash
DATABASE_URL=postgres://lorehaven:***@127.0.0.1:55432/postgres cargo test -p lorehaven-app retention
```

New tests, one per refusal path and named for the property:

```text
an_aggregate_instance_refuses_a_body_from_each_of_the_six_paths
every_refusal_names_the_instance_policy_rather_than_failing_generically
an_operator_may_narrow_a_source_and_may_not_widen_it
an_aggregated_work_is_found_by_metadata_and_not_by_a_body_only_search
an_aggregated_work_offers_no_offline_download_and_no_export
a_body_that_fails_to_fetch_leaves_a_retryable_import_and_never_a_silent_link
widening_to_cache_does_not_retro_fetch_and_narrowing_does_not_delete
works_past_saving_counts_only_works_this_instance_holds_no_body_for
works_past_saving_is_not_reachable_by_a_non_operator
```

**The test that matters most** is the last-but-one in that list, and it is the
one to write first: `a_body_that_fails_to_fetch_leaves_a_retryable_import`.
§11.15's non-degradation rule is what stops a temporary source failure becoming
a permanent loss, and it is a rule about a *failure path*, which is exactly
where a green suite lies.

---

## Phase D — preservation targets, verification, credits and clawback

Amendment §2, §3. Third migration, and the phase with the most moving parts.

### D.1 Migration 0087, both dialects

```sql
CREATE TABLE IF NOT EXISTS preservation_destinations (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    base_url            TEXT NOT NULL,
    match_rule          TEXT NOT NULL,   -- how one of its item pages is identified
    accepts_automated   INTEGER NOT NULL DEFAULT 0,
    enabled             INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1
);

-- The preservation target IS an identity member. Phase A0's
-- story_identity_members is where "this work also exists at that location"
-- lives, so preservation adds STATE to that row rather than a second table
-- saying the same thing in a different vocabulary. The 1:1 with
-- preservation_destinations is enforced so a destination maps to one member.
ALTER TABLE story_identity_members
    ADD COLUMN destination_id TEXT REFERENCES preservation_destinations (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members
    ADD COLUMN state          TEXT NOT NULL DEFAULT 'unverified';  -- unverified|verified|dead|refused
ALTER TABLE story_identity_members
    ADD COLUMN verified_at    TEXT;
ALTER TABLE story_identity_members
    ADD COLUMN dead_at        TEXT;
ALTER TABLE story_identity_members
    ADD COLUMN evidence_hash  TEXT;   -- hash of the destination page's identifying fields
ALTER TABLE story_identity_members
    ADD COLUMN credits_paid   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE story_identity_members
    ADD COLUMN created_by     TEXT REFERENCES accounts (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members
    ADD COLUMN updated_at     TEXT;
ALTER TABLE story_identity_members
    ADD COLUMN version        INTEGER NOT NULL DEFAULT 1;
CREATE UNIQUE INDEX idx_identity_member_destination
    ON story_identity_members (destination_id) WHERE destination_id IS NOT NULL;
```

**Why the target is a member and not its own table.** A `preservation_targets`
table and a `story_identity_members` table would both answer "does this work
exist at that location, and what is its state". Two tables, one fact, and the
reconciliation lands on whoever builds identity properly next — at which point
the preservation data has to be migrated, not just joined. The cost of the
ALTER-column shape is that `story_identity_members` now carries columns only
Phase D uses, and the benefit is that the concept exists once.

`credits_paid` sits here because the credit is owed to *a destination holding
this work*, which is the member. The clawback finds it by
`(destination_id, state)`; a dead member is a dead destination.

The partial unique index is load-bearing: a duplicate crosspost is an
idempotent no-op rather than a second paid target, and one destination cannot
back two member rows. `preservation_destinations` is `RESTRICT` because
deleting a destination must not silently orphan paid credits.

A0's `created_at` stays NOT NULL and D's `updated_at`/`version` are added
afterwards; **the ALTER block must be idempotent-safe for a fresh database**,
so test a migration against a database that has A0 and one that does not.

### D.2 The verification fetch is a `Metadata` fetch

This is the whole reason Phase A exists. A preservation check fetches the
destination's public item page as `FetchClass::Metadata` — robots, pacing and
the 1 MiB ceiling all apply — and compares title + author against the work's
identity. No destination API is required, which is the point: an archive that
has no adapter is still verifiable, and §2.5's eligibility calculation depends
on destinations being checkable without an adapter.

### D.3 Credits and the clawback

`db::economy::post_transaction` already takes an `idempotency_key` and replays
rather than double-posting, so the grant is:

```rust
economy::post_transaction(
    db, TxnType::Preservation,
    &format!("preservation:grant:{}", target_id),   // idempotency
    &target_id,
    &[(account, "credits", amount)],
).await?;
```

and the clawback is a second transaction with
`&format!("preservation:reclaim:{}", target_id)`. **Two entries, never a
balance edit** — the amendment requires the reader who already spent the
credits to see the debt, and a balance mutation cannot show that.

The re-check is a new `JobKind`:

```rust
#[doc = "Re-verify preservation targets and reclaim credits for dead ones (spec §11.12a)."]
PreservationRecheck => "preservation_recheck",
```

added to the `job_kinds!` tree in `crates/domain/src/jobs.rs`, so `ALL_KINDS`,
`as_str` and `parse` pick it up in the same edit, and `kind_index` becomes a
compile error until it is assigned a resource class. Scheduled from
`Worker::maintenance_pass` alongside the existing maintenance tasks.

### D.4 Sizing

`threshold`, `decay_bp`, `cap` are config. **Use integer arithmetic.**
`decay_bp = 2500` means 25%, and the marginal value is
`full * decay_bp / 10_000` — the same basis-points convention §0.4.6's
`theme_dial_floor_bp` and `tag_gravity_bp` already use, and for the same
reason: the repository has already been bitten by SQLite lacking math
functions (see the M58 note in `docs/verification.md` about
`dbg8c69 docs(votes): an integer exponent, because SQLite has no math
functions`). Compute in Rust, never in SQL.

### D.5 Leaderboard and badge

`Top Preservers` in §9.7.5's weekly categories, metric = **distinct targets
in state `verified`**, never actions performed and never all-time.
`badge_preservation` in §9.7.6's milestone badges, once ever, awarded at
`threshold` where that many eligible destinations exist (§2.5's floor of 1).

The badge eligibility calculation is a real function with a real reason
string, because "why can I not earn this" is a question the reader will ask:

```rust
pub enum PreservationEligibility {
    Reached { verified: usize },
    NoFurtherEligibleDestination { verified: usize },  // the floor-of-1 case
    Short { verified: usize, threshold: usize },
}
```

### D.6 The permission gate, and the imported-work refusal

**§33.1 is spec-only** — §33 opens with "Nothing in this section is
implemented" — so the `redistribution` assertion is a **dependency of this
phase, not an existing column**. Migration 0087 (or a small 0087a alongside
it) adds `works.redistribution TEXT NOT NULL DEFAULT 'unstated'`, checked against
`yes | ask | no | unstated`, editable only by the owning pseud. Check whether a
`works` column of that name already exists before assuming it does not:

```bash
rg -n 'redistribution' migrations/ crates/db/src/content.rs
```

`yes` pays full, `ask` pays less, `no` refuses by name. An `unstated` work is
treated as `ask` — the default must be the cautious one, because the reward
structure is what makes an unstated work worth asserting on, and an assertion
nobody has to make is an assertion nobody makes.

The **imported-work refusal** is separate and comes first: if the work is
imported (has a `source_key` that is not this instance) the crosspost is
refused with an error naming the missing §23.7 transfer manifest. This is a
refusal *by design*, and its test is the one that keeps it from being "fixed"
later by someone who reads it as a bug:

```text
a_preservation_target_for_an_imported_work_is_refused_naming_the_transfer_manifest
```

Add a comment at the call site saying the same thing, or the next reader will
helpfully implement it.

### D.7 Verification

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-app preservation 2>&1 | tail -40
```

New tests:

```text
reaching_the_threshold_awards_the_badge_once_and_never_again
a_fourth_destination_pays_the_decay_and_the_cap_pays_nothing
a_destination_that_stops_answering_is_marked_dead_and_reverses_its_credits
a_reversal_is_a_second_ledger_entry_and_never_a_balance_edit
a_replayed_grant_does_not_pay_twice
a_crosspost_to_a_destination_with_no_verified_record_pays_nothing
a_work_with_fewer_eligible_destinations_reports_no_further_eligible_destination
redistribution_no_refuses_and_ask_pays_less_and_yes_pays_full
a_preservation_target_for_an_imported_work_is_refused_naming_the_transfer_manifest
a_dead_preservation_never_marks_the_work_unreachable
the_top_preservers_metric_counts_live_targets_and_not_actions
```

The clawback tests are the ones to write first in this phase. Everything else
here is additive; the clawback is the only mechanism that stops the feature
being farmed, and a reward system without it is a spam vector with a badge.

---

## Phase E — retention proposals

Amendment §5, §19.15. Last, and it depends on C being real: a vote about a
setting that does not exist is a vote about nothing.

### E.1 Migration 0088, both dialects

```sql
CREATE TABLE IF NOT EXISTS retention_proposals (
    id              TEXT PRIMARY KEY,
    proposed_mode   TEXT NOT NULL,   -- cache | aggregate
    source_key      TEXT,            -- NULL = the instance-wide setting
    rationale       TEXT NOT NULL,
    opened_by       TEXT NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    closes_at       TEXT NOT NULL,
    state           TEXT NOT NULL,   -- open | passed | failed | overridden | expired
    tallied_at      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

-- A reader has one ballot per proposal. UNIQUE(account_id, proposal_id) is the
-- whole anti-buy mechanism: a second vote updates the first, so a vote cannot
-- be stacked.
--
-- NO WEIGHT COLUMN. §45.2 is explicit that governance votes are flat -- "every
-- vote weighs 1. Taste affinity, trust level, and private preference are never
-- part of governance" -- so a weight here would let a reading habit set instance
-- policy. An earlier draft of this plan carried weight_bp off §16.16's demand
-- weight; that inverted the arrow §19.2 refuses and was removed. The cost
-- direction is handled by quorum_for() instead: one reader is one ballot, and a
-- more expensive change needs more ballots.
CREATE TABLE IF NOT EXISTS retention_proposal_votes (
    proposal_id  TEXT NOT NULL REFERENCES retention_proposals (id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    support      INTEGER NOT NULL,   -- 1 | 0
    cast_at      TEXT NOT NULL,
    PRIMARY KEY (proposal_id, account_id)
);

CREATE TABLE IF NOT EXISTS retention_policy_changes (
    id          TEXT PRIMARY KEY,
    from_mode   TEXT,
    to_mode     TEXT NOT NULL,
    source_key  TEXT,
    actor       TEXT NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    reason      TEXT NOT NULL,
    decided_at  TEXT NOT NULL
);
```

**Do not add a weight column to this table.** If a future draft wants one, the
argument it has to beat is §45.2's flat-weights sentence, quoted above in the
migration comment so it is read before the column is written rather than after.
The M58 directory vote's `base_weight` is **not** a precedent for this: that is
a ranking signal over entries, and §45.2 governs governance only — a directory
ranking is not a governance vote. Conflating the two is how the wrong column
gets justified.

**This table is the privacy surface and it is the one to get right.**
`retention_proposal_votes` has no route that returns a row from it. The
tally is a separate aggregate query. A test asserts that no route in
`routes/retention.rs` can serialise a ballot — the shape of that test is
"a second account GETs the proposal and the response contains no `account_id`
key", which is a property rather than a code-reading.

### E.2 Binding mode

```text
POST   /api/v1/retention/proposals              (>= retention.proposal_min_trust)
GET    /api/v1/retention/proposals              (list; tally only)
GET    /api/v1/retention/proposals/:id
POST   /api/v1/retention/proposals/:id/vote
POST   /api/v1/admin/retention/proposals/:id/respond   (operator, advisory mode)
POST   /api/v1/admin/retention/proposals/:id/override  (operator, any mode)
```

The asymmetry (§5.3) is one pure function, and it is where the whole design
lives:

```rust
/// The quorum a retention change needs, by cost direction.
///
/// Narrowing storage is cheap and reversible. Widening it commits storage and
/// bandwidth indefinitely and is refused at the higher bar -- the default is
/// §19.4's "high-impact" bar of three.
pub fn quorum_for(proposed_mode: BodyMode, current: BodyMode, widen_quorum: i64) -> i64
```

`binding` commits **after** `cooling_days`, not at quorum, via a maintenance
task. A test freezes the clock and asserts the setting is unchanged at quorum
and changed at quorum + cooling.

### E.3 The §29.4 prerequisite

Wire `roadmap.min_trust` and replace the three `if trust < 1` sites in
`crates/app/src/routes/roadmap.rs` with the configured value, defaulting to 1
so no existing instance changes behaviour. §29.4's sentence then becomes true
and the amendment's citation of it stops being a lie.

```bash
cd ~/code-local/rust/lorehaven
cargo test -p lorehaven-app milestone_45_roadmap 2>&1 | tail -20
```

### E.4 Verification

```bash
cargo test -p lorehaven-app retention_proposal 2>&1 | tail -40
```

New tests:

```text
a_reader_below_the_bar_cannot_open_or_file_a_proposal_and_is_told_the_bar
in_advisory_mode_a_passed_proposal_does_not_change_the_setting
in_advisory_mode_the_operator_response_is_required_before_anything_moves
in_binding_mode_a_widening_proposal_at_the_ordinary_quorum_does_not_commit
in_binding_mode_a_widening_proposal_commits_after_quorum_and_cooling_days
a_narrowing_proposal_commits_at_the_ordinary_bar
an_operator_override_sets_the_setting_back_and_leaves_the_tally_visible
no_reader_can_read_another_readers_retention_ballot
a_second_vote_from_the_same_account_updates_the_first
a_proposal_payload_carrying_a_non_retention_field_is_refused_by_name
a_second_proposal_on_the_same_setting_is_refused_while_one_is_open
opening_a_proposal_grants_no_trust_credit_badge_or_placement
```

`opening_a_proposal_grants_no_trust_credit_badge_or_placement` is the one that
protects the feature from itself. A preference poll that made its participants
more visible would become a status ladder within two releases, and §19.15's
"a vote never grants the proposer anything personal" is cheap to state and
expensive to retrofit.

---

## Phase F — docs, ledger, and the checklists

Follows the phases, in the same commit as the last of them.

- `docs/requirements.csv`: one row per new requirement, ids `M59-01…M59-3x`,
  each with the test that pins it in `evidence` (not a bare file path — the
  README's §2.6 rule) and the reason it exists in `notes`.
- `docs/verification.md`: a newest-first section per phase, stating what was
  run and what the counts were. **Both dialects**, and if PostgreSQL was not
  run, say `implemented but not executed` — that is the vocabulary
  `docs/verification.md` §2.6 exists for.
- `docs/config-reference.md`: the `[retention]` and `[preservation]` blocks are
  new; `[imports] robots_posture` replaces `honour_robots` in the table.
- `docs/plans/README.md` and `docs/plans/remaining-work.md`: M59 gets a row,
  and the "where the build stands" counts in `remaining-work.md` are re-derived
  rather than carried forward (the status-drift note at the top of that file
  says so).
- `lorehaven.toml.example`: the new blocks, commented.
- `docs/spec-amendments/crawling-retention-and-preservation.md`: update the
  status line from `Final plan` to whatever the build actually reached, and
  strike nothing. An amendment whose decisions were partly not built should
  say which.

---

## Order and why

```text
A0 cross-source identity minimum      ← migration 0085; PREREQUISITE of D
A  robots posture + fetch class       ← no migration, amends working code
B  class-specific UA token            ← same config surface as A
C  retention built (M6-15)            ← migration 0086; the setting everything else is about
C1 body audience (§7.7)               ← no migration of its own; rides C's 0086
D  preservation targets + rewards     ← needs A0's member row, A's class, C's setting
E  retention proposals                ← migration 0088; needs C to be real before there is anything to vote on
F  docs + ledger                      ← with the last phase
```

C1 is **inside** C rather than after it: the audience is a field on the same
`works` / retention rows and a new arm of the same eligibility function, so
splitting it invites two writers in `policy.rs` at once. It carries no migration
of its own — `works.body_audience` lands in C's 0086.

A and C can run in parallel by different hands. D and E both touch
`routes/retention.rs` and the admin surface, so they serialise.

**A0 was added on 2026-09-27 after a second probe found that none of §11.10's
four identity tables exists.** The first draft of this plan treated Phase D's
ground as empty, which it is — but the *concept* was not missing, only the
implementation, and the concept is what Phase D hangs a destination on. A
destination is an edition member (§11.10b), so `preservation_targets` becomes
the state half of a `story_identity_members` row rather than a parallel concept
that a later identity milestone has to reconcile.

Migration numbers shifted by one when A0 was inserted, and by one more when
A0 was built. The `job_kinds!` tree and the migration-id parity test are the
only things that care about order, and neither is a reason to leave a plan
claiming a number the repository has already used.

| phase | first written as | after A0 was inserted | as built |
|---|---|---|---|
| A0 identity | 0084 | 0084 | **0085** |
| C retention | 0085 | 0086 | 0086 |
| D preservation | 0086 | 0087 | 0087 |
| E proposals | 0087 | 0088 | 0088 |

**0084 belongs to something else and always did.** Migration 0083 (M54, shipped)
reserves it for the `bot_registrations.token_id` foreign key — gap D5 — and says
so in its own header: the constraint cannot ride in 0083 because SQLite cannot
`ALTER TABLE ... ADD CONSTRAINT` and PostgreSQL needs a UUID-to-TEXT column type
migration first. A0 was written before 0083 shipped and took a number that was
already spoken for. The collision is recorded here rather than papered over,
because a plan that names a number twice is worse than one that names the wrong
number: the first makes the second migration's author believe their number is
free.

**What is deliberately not in this plan.** §11.11 preservation *batches*
(M6-10) stay unbuilt: they need an operator role, a permission basis, a dry-run
report and a batch rollback, and none of that is needed for a preservation
*target*. Building the target first means the batch has a destination-side to
attach to when it arrives. §23.7's transfer manifest is likewise untouched —
Phase D refuses the imported-work case rather than quietly depending on it.

**What A0 does not build.** `identity_merge_proposals` and
`identity_merge_history` are *created by* A0 (empty, so §3's data model is
honest and the roadmap seeds them) but nothing writes to them. Merges, the
six-way classifier, evidence, quorum review and private grouping are §11.10's
full scope and belong to their own milestone. `edition_relation` ships with one
value, `cross_posted`, because that is the only one this instance can know for
certain — it performed the act. A `translation` recorded as `cross_posted`
tells a reader two texts are one, and that error is the one §11.10 exists to
prevent.

---

## Gate discipline

From `docs/plans/remaining-work.md`, and every one of these has bitten:

```bash
cd ~/code-local/rust/lorehaven
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3   # must be 0/0
cargo fmt --all -- --check
cargo test --workspace --no-fail-fast --test-threads=2 2>&1 | tail -8
```

- **Redirect, then read the file.** Never pipe through `grep -c` without
  `2>&1`; the count lies when the build fails first.
- **Run the new suite twice.** A fixture that names a fixed id passes exactly
  once against a persistent dev database and dies in the full suite. The
  `crates/app/tests/` harness creates its own directory per fixture — keep it
  that way and do not introduce a shared one for preservation rows.
- **Both dialects or neither.** Every phase here has a migration except A and
  B, and A/B change security-relevant code, so A and B are verified on both
  backends too.
- **If a gate has never failed, suspect the gate.** The M54 handoff's
  `m57_metadata_exchange` note records a test asserting on error *text* that
  passed for the wrong reason. Assert on the error **type** and the reason
  code; prose goes in the assertion message.
