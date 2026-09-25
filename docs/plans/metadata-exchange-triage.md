# External AI proposal triage — metadata exchange + curation pipeline

**Status: reviewed and adopted 2026-09-25 (Alvaro).** This began as a review
draft of a chatbot proposal pasted 2026-09-25. The adopted items are now landed
in `docs/spec.md` (§0.3, §2.3.1, §11.17, §15.17, §16.16.1, §19.14) with
requirement rows in `docs/requirements.csv` and a build order in
`docs/plans/remaining-work.md` (M57). The rejected items below stay rejected,
with their reasons — a rejection that is not written down is a rejection the
next session re-proposes.

The section numbers in the correction table were correct as of 2026-09-25 and
**§15.17 is the free number in ch15, not §15.16** — §15.16 was already Content
notes when this was written, so the table's own recommendation was one number
stale. Fixed in the applied landing. Everything else landed as tabulated.

Two documents were under review:

- **A** — the Tauri ebook library spec (vault:
  `~/secondbrain/10-Projects/2026-09-25T130540+0200-tauri-ebook-library-spec.md`)
- **B** — this repo's `docs/spec.md`

## Verdict

**The central idea is right and worth adopting. The diffs as written are not
applicable — roughly a third of B's section citations are wrong, and two of them
name sections that already exist with different content.** Adopt the curation
pipeline into A, adopt the exchange protocol into B in corrected form, and
discard the rest.

The load-bearing claim in the proposal is correct and I verified it: **the
spec's §5.6 excluded metadata fetching, and that exclusion is wrong for this
user.** A pile of files with no metadata is the actual use case, and an app that
organises files you have to identify yourself is a better Calibre, not a
curator. The proposal is right that this changes the shape of the product.

The proposal is also right that a curation pipeline and a community metadata
exchange are one idea seen from two ends, and that they should share a wire
format. That is a real architectural insight, not a restatement.

## What the proposal got right

1. **§5.6's "no scraping" is the wrong call.** Distinguishing *metadata backfill*
   (the file exists, answer "what is this?") from *content download* (fetch
   chapters) is a correct and important distinction. The spec's original
   position refused a harmless operation on the grounds that a related one was
   out of scope.
2. **Version resolution beats duplicate-flagging.** §3.4 rule 3 says same
   identifier + different hash → flag for review. For serialised fiction that
   is the *normal lifecycle*, not an anomaly, and a review queue that fires 400
   times will never be cleared. "Keep newest, archive older with
   `superseded_by`" is correct.
3. **The inbox is the difference between auto-curate and auto-guess.** A
   first-class triage view with the app's best guess and one-click accept is the
   right shape, and the target ratio (4,000 files → 50–200 inbox items) is a
   falsifiable design commitment rather than a vibe.
4. **Metadata exchange as a separate concern from analytics.** The proposal
   insists these are independently controllable, and is right. Shipping
   "instance sync" inside the analytics consent model would have been a
   category error, and it is a mistake worth naming explicitly.
5. **The shared wire crate.** `lore_metadata` as a dependency-light contract
   crate compiled against by both sides is the correct way to keep a protocol
   from rotting. It should be published early — the proposal's note that it can
   be written during M0/M1 regardless of everything else's order is right.

## Corrections required before any of B's diff is applied

The proposal cites sections by number. Those numbers were not verified against
the document. Checked with `grep -nE '^#+ *N.M' docs/spec.md`:

| Proposal cites | Actually is | Consequence |
|---|---|---|
| §11.16 (exchange endpoint) | **§11.16 Adapter porting backlog (ADR 0024)** | Collision. §11.17 is the next free number in ch11. |
| §19.8 (trust for exchange) | **§19.8 Bootstrap mode** | Collision. Trust chapter runs to §19.13. |
| §14.5, §14.6 | **Do not exist** (ch14 ends at §14.3) | Invented. |
| §16.18 | **Does not exist** | Invented. §16.16 is the last in ch16. |
| §15.15 (signal taxonomy growth) | **§15.15 Length histogram in search** | Collision. §15.16 is the last in ch15. |
| §5 (milestone amend) | no `## 5.` heading of that shape | Milestones are numbered differently; needs reading. |
| `entity_alias` | `entity_aliases` | Wrong table name. |
| `story_identities` fields "canonical metadata, visibility, status" | correct | One of the few accurate citations. |

So **B's diff cannot be applied as a diff.** It needs to be re-authored as
anchored edits against real section text, in the style of
`plan-document-maintenance/references/spec-section-authoring.md`.

The proposal's table names, though, are mostly real: `canonical_entities`,
`entity_aliases`, `tag_proposals`, `metadata_suggestions` and `story_identities`
all exist in `docs/spec.md` §4.5 and §4.3. The substrate is not invented — which
matters, because it means the exchange endpoint is a new *input* to an existing
governance system rather than a parallel one.

## The milestone-ordering claim is wrong

The proposal says M6.5 "depends on M10 (taxonomy) and M14 (trust) … In practice
this means it lands in the M15–M16 range of the build order, which is late — but
it cannot be earlier without the taxonomy and trust infrastructure."

Both prerequisites are **already built**. `docs/requirements.csv` is ground
truth (248 rows: 172 `implemented-locally-tested`, 20
`implemented-fully-tested`, 52 `planned`, 4 `unsupported`):

- M10 taxonomy — 6 rows, all `implemented-locally-tested`
- M14 trust — 1 row, `implemented-locally-tested`
- M6 imports — 13 rows `implemented-locally-tested`

So the sequencing argument does not hold. The real constraint is not
sequencing, it is **verification depth**: 172 of 248 rows are
`implemented-locally-tested` rather than `implemented-fully-tested`, and M10
taxonomy sits in that majority. Building an unauthenticated-shaped public write
path on top of machinery that has never been exercised through the full stack
is the actual risk, and it is a different risk from the one the proposal names.

The genuinely open work is elsewhere: **46 of the 52 planned rows are M45**,
plus 2 each for M52/M53/M54. None of that is metadata exchange.

---

## Classification, item by item

Per `plan-document-maintenance`: adopt / modify / reject / defer, with the
conflict named for each rejection.

### Adopt

| Item | Lands in | Note |
|---|---|---|
| Curation pipeline §3.6 (six stages) | A §3.6 | The core correction. Identify → resolve → enrich → deduplicate → classify → inbox. |
| Inbox as first-class view §3.7 | A §3.7 | With the 10%-of-scan tuning rule, which makes it falsifiable. |
| Anthology splitting §3.8 | A §3.8 | EpubSplit as a pipeline stage, inbox-gated. |
| Version resolution replacing flag-for-review | A §3.4 | The single most important behavioural change for serialised fiction. |
| Metadata backfill in scope | A §5.6 | Replaces the "no scraping" row. Narrowly bounded: catalog lookup only. |
| Metadata Backfill + Filename Parser as plugins | A §6 | Exercises `net.fetch`; site parsers become plugins not core. |
| Exchange is not analytics | A §9 | Two independent consent surfaces in settings. |
| Per-column send privacy | A §3.9.3 | `#notes`, `#read_progress`, `reading_status`, `#times_read`, `#last_read`, `#read_dates`, `#rating` private by default — user judgments, not work metadata. |
| `lore_metadata` shared crate | A §2.1, B §2.3 | Published early, serde-only, no IO. |
| Signal content prohibitions | B §0.3 | Enforced by schema, not trust. This is the strongest privacy idea in the proposal. |
| Exchange endpoints | B §11.17 (renumbered) | `signals`, `canonical`, `version`. |
| Auto-created entities usable immediately | B §15.16 (renumbered) | The "usable but unverified" split is right — it avoids a gate that stalls organic growth. |
| Latent demand | B §16.16 amendment | A signal for an unknown work is demand evidence. Genuinely additive. |
| Admin routes + public stats | B §24.1, §24.2 | Aggregate counts only, never per-user. |

### Modify

| Item | Change | Why |
|---|---|---|
| §19.8 trust for exchange | Renumber to **§19.14**, and reconsider the TL3+ bar | Trust chapter ends at §19.13. The *principle* is right (low bar to submit, high bar to curate) but the proposal asserts a specific level without reading what TL3 gates elsewhere. |
| M6.5 milestone | Renumber, and drop the false dependency claim | M10/M14 are built. The real constraint is verification depth (§ above). |
| `auto_quorum_threshold = 3` | Make it an instance parameter, and **do not call it quorum** | "Quorum" in this spec means human reviewer quorum (§19.4). Three bots agreeing is a *confidence* threshold, not a quorum. Naming it quorum would corrupt a load-bearing term. |
| Confidence scores (0.95, 0.3–0.6) | Placeholders, config, expect revision | Vibe-as-calibration. Per the skill's guidance: keep only where the mechanism cannot run without a number, publish the default, expect real data to move it. |
| "Instance metadata preferred over site scrape" | Keep, but make it a *suggestion* | Matches the proposal's own conflict rule — never silently applied. Consistent with spec's "suggestions do not become assertions without approval" (§4.5). |
| Anthology false-positive tolerance | Defer the decision | The proposal asks the user; §3.7's inbox-gating makes the question answerable with data instead. |

### Reject

| Item | Conflict |
|---|---|
| Credits for signal submission (§20.3) | **Contradicts §19.2 and the §0.3 credits/trust separation.** "Earning credits never advances trust level" — and paying credits for metadata submission is precisely the activity-for-reward pattern the spec refuses for trust. The exchange needs no incentive: it is opt-in and the user benefits directly. Also creates a spam vector against a 1000/hour rate limit. |
| `signal_count` as a demand signal feeding §16.16 | **Rejected as specified, adopt a weaker form.** §16.16 demand weight is per-reader pull on unwritten content; an aggregate "N users have this" is a different quantity and the proposal's own privacy invariant says the instance cannot count users. `signal_count` cannot distinguish one user submitting twice from two users once, so it is not a count. Usable as a *review-priority* signal (B §15.16), not as a demand weight. |
| Publishing the app's "three signals agree" as auto-quorum | Terminology corruption, see Modify. |
| Applying instance metadata to the local library automatically | Violates the proposal's own stated rule three paragraphs earlier. Inconsistent within the proposal itself. |

### Defer

Named explicitly so deferral reads as a decision:

- **Re-downloading updated chapters.** The proposal asks the user. This is
  FanFicFare's job and the boundary A §5.6 already draws. The app knows a fic
  has been updated (A §7 stale-mark); it does not fetch. Keeping this deferred
  also keeps the "this is a library manager, not a downloader" line intact.
- **Content downloading** into A — same reason.
- **Native mobile client** — unchanged from A §10.
- **Third-party non-Lorehaven sync targets** — the protocol should not name
  Lorehaven as the only implementer, per the skill's vendor-agnostic rule. A
  compatible third party is fine; a spec that assumes one server is not.

## The one design conflict worth arguing about

The proposal wants instance-supplied canonical metadata to be *preferred over*
direct site lookup, on the grounds that it "has passed quorum review; the site's
has not." That reasoning is sound. The mechanism is not, and the proposal's own
§3.9 conflict rule already contradicts it: instance metadata is offered as a
per-field suggestion, never applied silently.

Both are right, and the resolution is ordering, not preference: the instance
value is presented first, with its provenance and quorum status visible, and
the site value shown as the alternative. The user accepts. This is the same
pattern as the tag-proposal queue in §4.5, and reusing it means the exchange
needs no new review machinery — it feeds the queue that already exists.

## Verification block

Run after any future application of this triage:

```bash
# new sections, one count each
grep -c '^## 11.17' docs/spec.md          # expect 1
grep -c '^## 15.16' docs/spec.md          # expect 1
grep -c '^## 19.14' docs/spec.md          # expect 1
# no collisions introduced
grep -cE '^## 11\.(1[7-9])' docs/spec.md  # expect 1
grep -cE '^## 15\.16' docs/spec.md        # expect 1
# forbidden vocabulary: "quorum" must not describe signal counting
grep -n 'auto_quorum' docs/spec.md        # expect nothing
# code fences balanced
awk '/^```/ {c++} END {print c, (c%2==0?"balanced":"UNBALANCED")}' docs/spec.md
```

## Correction rule

A statement here found to be wrong is fixed in place, in the same commit as
whatever proved it wrong. Section numbers in this document will rot as B is
edited — re-verify with the `grep` in §"Corrections required" before citing any
of them. An ambiguous decision becomes an ADR, not a comment.

