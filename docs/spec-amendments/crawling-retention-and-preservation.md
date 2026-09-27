# Lorehaven Spec Amendment — Crawling Posture, Retention Governance, and Preservation

**Status:** Final plan
**Date:** 2026-09-27
**Source:** Owner brainstorm (robots flexibility, preservation rewards, trust-gated body visibility, community-voted retention)
**Amends:** §11.5, §11.11, §11.12, §11.15, §9.7.1, §9.7.2, §9.7.5, §9.7.6, §19.1, §28.2, §43
**Adds:** §19.15, §11.12a, §11.15a, §11.15b
**Milestone:** M59
**Plan:** `docs/plans/crawling-retention-preservation.md`

---

## Overview

This amendment answers three adjacent questions that the spec currently answers
in three disconnected places, and it answers them in one vocabulary:

1. **What may this instance read from a source it was told not to read?**
   §11.5 answers with a boolean and an operator override.
2. **Does this instance keep the words of an external work?**
   §11.15 answers with `cache | aggregate` and an operator setting.
3. **Who gets a say in (2), and what are they rewarded for keeping a work
   alive somewhere else?**

The three are one subject. What an instance may read determines what it can
keep; what it keeps is what its readers can read without leaving; and the
community that reads it has an interest in both. Split across three sections
they contradict each other — §11.5's override silently restores bytes that
§11.15 says this instance does not hold, and nothing anywhere rewards the
behaviour the platform's own premise depends on.

### The decisions, and what each one costs

| # | Decision | Cost |
|---|---|---|
| 1 | `imports.honour_robots: bool` becomes `imports.robots_posture: strict \| metadata_only \| permissive`, and every fetch carries a declared class | A third posture is a third thing to get wrong, and a metadata fetch that returns a body is the new failure mode. §1.3 is the whole defence. |
| 2 | The reward for preserving a work is **credits plus a badge**, not points | §28.10 refuses points and levels by name. Credits are spent and a badge records one occurrence, so both halves of the request survive and the refused half is refused for the reason already written down. |
| 3 | The reward attaches to **verified live preservation**, not to the act of posting a link, and a destination that dies **claws the credit back** | More machinery than a click would need. It is also the only mechanism that makes the reward un-farmable, and §11.13's vanished-source treatment gains a second job. |
| 4 | A trust-level-gated **view** of a body on a caching instance is refused; a trust-level-gated **request** for a personal snapshot is provided | The trusted reader gets the bytes, so little is lost. The refused version is recorded here with its reason so a later draft does not re-propose it. |
| 5 | Retention may be decided by a community vote, in `advisory` (default) or `binding` mode | In `binding` mode a vote can move an operator's storage bill and an instance's preservation debt. It is opt-in, it is asymmetric in cost, and the operator retains a recorded override. |
| 6 | Cross-posting an **imported** work to a preservation target is refused as unimplemented | The permission chain that would make it safe does not exist. §23.7 defers the transfer manifest; this amendment does not quietly un-defer it. Local works are not blocked, and they are the shippable slice. |

### What already exists, so nothing here is invented

- §11.5 already applies a group naming our `User-Agent` token ahead of `*`,
  and the live-verification suite already records the case that makes this
  amendment necessary: Wattpad's story document is permitted and its prose is
  under `Disallow: /apiv2/*`. **On most sources the metadata paths are already
  the allowed ones.** That is currently a property of URLs rather than a
  policy, which is why the split is worth stating rather than leaving to
  luck.
- `FetchPolicy::honour_robots` exists in `crates/scrapers/src/safety.rs` with
  `robots_gate` as a pure function, a per-host override counter and a
  first-per-host log. §1 amends a working mechanism rather than replacing one.
- §11.15's `cache | aggregate` is fully specified and entirely unbuilt
  (`requirements.csv` M6-15, `unsupported`: no setting, no refusal path, no
  admin route). §4 builds it.
- §29.4 already makes a community vote's trust gate **configurable** and
  already splits the authority: the community advises, the operator decides.
  §5 extends that split to a second, opt-in mode.
- §16.16's demand weight is already a reader's pull on unwritten content, and
  §11.13 already marks a vanished source rather than deleting it. §3 and §4.4
  reuse both.

---

## §11.5 Fetching safety — Robots posture and the fetch class

**Modification.** The `Disallow` override stops being a boolean and becomes a
three-valued posture, and every fetch declares what kind of fetch it is.

```text
imports.robots_posture = strict | metadata_only | permissive   # default: strict
```

| Posture | `Disallow` on a metadata fetch | `Disallow` on a content fetch | `Crawl-delay` and the 1s floor |
|---|---|---|---|
| `strict` | refused | refused | always enforced |
| `metadata_only` | honoured **by not storing it** — read, classified as metadata, discarded | refused | always enforced |
| `permissive` | read and stored | read and stored | always enforced |

`imports.honour_robots` is retained as a compatibility key, not as a second
source of truth: `honour_robots = false` reads as `permissive` and
`honour_robots = true` reads as `strict`. When both keys are present the
posture wins, and the resolved value is what `/api/v1/meta` and the source
catalogue entry report. The key is deprecated in the operator docs and may be
removed once no supported configuration names it.

### 1.1 The fetch class is declared by the adapter and checked by the fetcher

```text
FetchClass = metadata | content | media
```

The class is set **at the call site, by the adapter that declares it**, and
never inferred from a URL. A URL that looks like a chapter page fetched as
`metadata` is still a `metadata` fetch and is bounded as one. Inference from
shape is how a "just the metadata" door becomes a body door, because the
adapter's guess and the fetcher's ceiling then disagree.

`robots_gate` takes the class as a fourth input and remains a pure function of
`(rules, path, policy, class)`, which is what keeps it testable against real
parsed rules with no network:

```text
robots_gate(rules, path, posture, class) ->
    Allowed | Refused | ReadAndDiscarded | Overridden
```

`ReadAndDiscarded` is the new answer and exists only on `metadata_only`. It is
distinct from `Allowed` because the bytes are not kept, and distinct from
`Overridden` because nothing was overridden — the posture asked for the read
and then declined to keep it.

### 1.2 A metadata fetch cannot return a body

Three independent limits, and all three are required, because each one alone
has a failure mode the others do not cover:

- **A byte ceiling far below a chapter.** `max_bytes` for a `metadata` fetch is
  a fraction of the content ceiling (1 MiB against 8 MiB today), so a source
  that answers a metadata request with the whole work is truncated and the
  truncation is visible rather than silent.
- **A parse target that cannot hold prose.** The `metadata` class is parsed
  into `WorkMetadata`, which has no field a body could arrive in. A body
  cannot be *returned* through a type with nowhere to put it — the §11.17
  precedent, which made the signal schema refuse a prohibited field so a
  non-conforming client could not send one.
- **No content-derived side effects.** A `metadata` fetch writes metadata rows
  only. It never writes a revision, never populates FTS, never enqueues a
  body-fill job, and never satisfies a cache fill.

**A `metadata` fetch is one whose *result* cannot be prose.** The ceiling stops
a large document; the type stops a small one; the write list stops a truncated
one. Three of them, because "we capped it" is not a reason to have missed the
other two.

### 1.3 `metadata_only` is a posture, not an exemption

The existing §11.5 limits on the override carry over unchanged, and three more
are added because a posture that reads what it was refused is a stronger claim
than a boolean that ignored it:

- **It is still not access-control circumvention.** The §11.5 prohibition is
  untouched: nothing here defeats a login, an age gate, a paywall, a challenge,
  or a page the host's own code gates. A `metadata_only` instance that cannot
  read a story page does not get to read it.
- **Every overridden or discarded read is counted, and the first per host is
  logged**, exactly as today. The counter is what turns "we don't crawl them"
  into a number an operator can answer a question with.
- **A `content` fetch against a refused path is a hard refusal naming the
  posture**, and it is not retryable. A retry would be the same refusal
  re-running every backoff interval, which is a way of converting a policy into
  load.
- **A source whose prose is entirely refused stays catalogued.** An instance
  running `metadata_only` holds the work's title, author, tags, completion
  status and reachability, and the work page says the text lives elsewhere
  because this instance's posture says so. That is a complete record, and
  §11.15's `aggregate` is what it is called.

### 1.4 Per-source posture is available and expires

§11.5 currently refuses a per-source switch outright, on the grounds that "a
per-source switch would let an override be made once and forgotten about for
the source it affects". The objection is to *persistence*, not to
granularity, so the granular form is allowed with a lifetime:

- A per-source-family posture may be set by the operator, may only **narrow**
  toward more caution than the instance posture, and is recorded in the modlog
  with who set it and when.
- It **expires with the import run that needed it** unless the operator extends
  it, so no override can outlive the job that justified it by default.
- It is reported on the source's catalogue entry (§11.5's visibility rule), not
  only in a log.

`imports.robots_posture` stays the single instance-level statement of posture,
which is what a reader on the instance sees.

### 1.5 The User-Agent token may name the class, and may not be chosen for the rules it lands in

A source can address its rules at a class of read rather than at a path. The
instance may therefore send a class-specific token — `Lorehaven/1.0 (+import;
class=metadata)` — which is honest, disclosed, and lets a site write rules it
means.

**What is refused:** choosing the token to land in the *more permissive* group.
A host that disallows `/data/*` for `*` and allows it for `SomeOtherBot` must
not be reachable here by claiming to be `SomeOtherBot`. A token is a truthful
description of the client, never a credential. An unrecognised or impersonating
token is an abuse finding under §24.5, not a configuration option.

### 1.6 Acceptance

- `strict` refuses a disallowed metadata path and a disallowed content path;
  `metadata_only` reads the first and refuses the second with a message naming
  the posture; `permissive` reads both.
- `Crawl-delay` and the one-second floor are enforced identically under all
  three postures, asserted by a test that runs the same host's rules through
  all three.
- A `metadata` fetch over a body-bearing response writes no revision, no FTS
  row, and no cache-fill job, and the metadata parse type has no body field.
- Every `ReadAndDiscarded` and `Overridden` read increments the per-host counter
  and the first one per host is logged.
- A `content` fetch refused on a `metadata_only` instance is not retried.
- A per-source posture narrower than the instance posture works, appears on the
  source's catalogue entry, and is gone after its import run ends.
- `honour_robots = false` alone yields `permissive`; both keys present yields
  the posture; the resolved value is what the API reports.

---

## §9.7 Gamification — credits and badges for verified preservation

**Modification.** The reward for keeping a work alive is **credits and a
badge**, sized by verified live preservation.

**Not points.** §28.10 refuses XP, levels, points and lifetime totals by name,
and §9.7.1 refuses anything that compounds. A "preservation score" that grows
with each archive a work reaches is exactly the shape both sentences were
written to refuse, and adding it would be a contradiction introduced by a
later section rather than a design choice. What replaces it is already in the
spec's own vocabulary: **credits**, which are spent, and a **badge**, which
records that something happened once (§9.7.6). Both survive the request; the
total does not.

### 2.1 The reward attaches to verification, not to the click

A crosspost is a request. A preserved work is a fact about the world that can
be checked, and only the fact is worth paying for.

```text
credit_preservation_verified      flat credits, daily-capped
badge_preservation                awarded once at the full threshold
```

Rewarding the act pays for spam: a link farm is cheap, and twenty farms is a
larger score than three real archives. Verification is:

- **A metadata-class fetch of the destination's public item page** through the
  ordinary §11.5 path — robots, pacing, byte ceiling — confirming that the
  destination carries a record naming the same work. No destination API is
  assumed and none is required.
- Recorded with `verified_at`, the destination host, and the evidence (a
  content hash of the destination page's identifying fields).
- Re-checked on a schedule. A destination that stops answering marks the
  preservation **dead**, and §2.2 takes the credit back.

### 2.2 Sizing: a threshold, a decay, and a cap

```text
[preservation]
threshold           = 3      # distinct verified destinations for the full reward
decay_bp            = 2500   # 25% of the marginal value per destination past the threshold
cap                 = 8      # destinations that count at all
```

- Reaching `threshold` distinct verified destinations pays the full reward and
  awards `badge_preservation`. **The badge is once, ever.**
- Past the threshold each additional verified destination pays
  `full × decay_bp` and decays geometrically, so a seventh archive is worth
  less than a sixth and none of them compounds.
- Past `cap`, a destination records the preservation and displays it, but pays
  nothing and does not count toward the threshold. A reader who finds an
  eleventh archive has still done a good thing; they are not paid more for it.
- All of it sits inside the existing §9.7.2 daily action cap, so preservation
  cannot be farmed by volume even if every limit above were wrong.

### 2.3 Clawback

A dead destination reverses the credits it paid, through a §20.1 ledger
reversal with an `idempotency_key` naming the preservation — never a balance
edit, so the reversal is auditable and cannot be double-applied.

This is the load-bearing piece of the whole section. It is what makes
"Top Preservers" (§2.4) a quality metric: a spammer's destinations die, the
records die with them, the credits go back, and the leaderboard corrects
itself without a moderator having to notice. Without it the leaderboard is a
count of how many link farms somebody registered this week.

A reversal never takes a balance negative without saying so: the ledger records
the credit and the reversal as two entries, so a reader who already spent them
sees the debt, and a reader who did not is unaffected.

### 2.4 Leaderboard and badge

- **`Top Preservers`**, weekly, alongside §9.7.5's existing categories. The
  metric is **distinct preservation destinations currently verified live**,
  never the number of crosspost actions performed. A destination that has gone
  dark stops counting, which is the same anti-farm property as the clawback,
  applied to the ranking. No all-time category exists (§9.7.5).
- **`badge_preservation`**, one-time and permanent, awarded at `threshold`
  distinct verified destinations on a single work (§9.7.6). Badge counts are
  never summed into a rank.
- **No composite preservation score is shown anywhere.** §32.6 refuses
  composite quality scores publicly by default. "Preserved in 3 verified
  archives, last checked 2026-09-20" is a fact with a date; a 0–100
  preservation number is a score, and it would be read as a ranking.

### 2.5 The threshold is not universally satisfiable, and says so

Some archives accept only original work. Some have no adapter and cannot be
verified. Three verified destinations does not exist for every work, so a
flat requirement is a number some readers can never earn and a failure that
looks like the reader's.

- The badge threshold is **`threshold` verified destinations where that many
  eligible destinations exist**, with a stated **floor of 1**. A work with one
  verified destination and no eligible second destination is `fully_preserved
  (no further eligible destination)`, not short.
- Eligibility is computed and shown, so a reader can see *why* the badge is not
  reachable for a given work rather than concluding it is unreachable.
- The default `threshold = 3` is the owner's stated value and stays the default.

### 2.6 Permission gates the reward

- The destination must be one the **author** permitted. §33.1's
  `redistribution` assertion gates the whole action: `yes` pays the full
  reward, `ask` pays a reduced amount, `no` pays nothing and the crosspost is
  refused by name. The incentive then points at the corpus that genuinely
  should be preserved, which is the point of having a permission model at all.
- An **imported** work's assertion is inherited from the origin, and if the
  origin says `no` or `ask` the crosspost is refused. Pushing site A's fic to
  site B is A→B redistribution, and it is the hardest case in the set. §2.7
  says why it cannot ship yet.
- A work's `redistribution` assertion may be set by the author and survives
  orphaning and account deletion (§33.1's existing rule).

### 2.7 Imported works are refused, and the reason is a deferred milestone

Enforcing inherited `redistribution` across an origin requires the **transfer
manifest** — provenance and permission travelling with a record between
instances — which §23.7 explicitly defers. Until that exists, an instance
cannot tell whether a foreign author permitted redistribution, so a
crosspost of an imported work would be an unverifiable permission claim.

**The refusal, in spec text, so it is not rediscovered as a bug:** a
preservation target for an imported work is refused with an error naming the
missing transfer manifest. Local works by this instance's authors are not
refused and are the whole of the shippable slice.

This is a real reduction from what the brainstorm asked for, taken knowingly:
the reader can preserve what this instance's authors wrote, and not what it
imported. It is the same order the spec already chose in §11.11, which puts
preservation batches behind a documented permission basis.

### 2.8 Acceptance

- Reaching `threshold` verified destinations awards the badge once; a second
  threshold on another work does not award it again.
- A fourth destination pays `full × decay_bp`, a fifth less, and `cap`
  destinations pay nothing.
- A destination that stops answering is marked dead, its credits are reversed
  by a ledger entry, and it leaves the leaderboard metric.
- A crosspost to a destination with no verified record pays nothing and awards
  nothing.
- A work with fewer eligible destinations than `threshold` is reported as fully
  preserved for what is eligible, with the reason.
- `redistribution: no` refuses the crosspost by name; `ask` pays a reduced
  amount; `yes` pays the full amount.
- A preservation target for an imported work is refused naming the transfer
  manifest.
- No surface anywhere shows a composite preservation score.

---

## §11.12a Preservation targets (outbound)

**Addition.** §11.12 publishes a work to an external site for readers to find.
A **preservation target** is the same mechanism pointed at the other purpose: a
destination that keeps a copy so the work survives if this instance or its
origin does not.

§11.12's existing rules are unchanged and bind in full: explicit destination
authentication, per-post confirmation with preview, per-work per-destination
status, failed crossposts not affecting the local work, and the same credential
vault. **A preservation target is a crosspost.** It is not a second, looser
door — it is §11.12 with §11.12a's verification and reward attached, and it is
reached through the same per-destination confirmation.

### 3.1 Why this is not "automatic cross-posting of every published work"

§11.12 refuses automatic crossposting, and this does not reopen it. A
preservation target is a per-work, per-destination action a person takes and
confirms. What is new is the verification loop and the reward, not an
automation nobody asked for.

### 3.2 Eligibility

A work is eligible for a preservation target when:

- the `redistribution` assertion permits it (§2.6), and
- the work is local to this instance (§2.7), and
- the destination is on a configured list of preservation destinations.

A **preservation destination** is instance configuration: a named archive, its
base URL, the adapter or match rule that identifies one of its item pages, and
whether it accepts automated submission at all. An instance with no
destinations configured has no preservation targets, and §2.5's eligibility
calculation reports zero eligible destinations rather than pretending otherwise.

### 3.3 Availability checking extends to destinations

§11.13's availability checking currently watches the work's **origin**. A
preservation target adds a second thing to watch, and the distinction matters:

```text
preservation_state = unverified | verified | dead | refused
```

`dead` means the destination no longer carries a record naming the work. It
does **not** mean the work is lost — the work's own reachability is a separate
fact and is reported separately. A work can be live here and dead in all three
of its archives, and an operator who sees only one number would be told the
wrong thing.

### 3.4 Aggregate instances are the case where this matters most

§11.15 refuses storing a body on an `aggregate` instance, and §11.11 refuses a
preservation batch there for the same reason. An aggregating instance is
therefore the one most dependent on somebody else holding the text — and
precisely the one that can do least about it. §11.15a gives the community a way
to say so, and this section gives them the tool to act on it.

`aggregate` + preservation targets is a coherent posture, not a contradiction:
the instance keeps the record and the community, not the instance, keeps the
words.

### 3.5 Acceptance

- A preservation target is a §11.12 crosspost: same authentication, same
  per-post confirmation, same status tracking, same vault.
- A destination with no verified record shows as `unverified` and pays nothing.
- A destination that stops answering becomes `dead`, the credits are reversed,
  and the work's own origin reachability is unaffected.
- A `dead` preservation never marks the work unreachable and never deletes it.
- An instance with no configured destinations reports zero eligible
  destinations in §2.5's eligibility calculation.

---

## §11.15 / §11.15a / §11.15b Retention: building the setting, the vote, and the per-reader request

§11.15 as written is complete and unbuilt (`requirements.csv` M6-15,
`unsupported`: no setting, no refusal path, no admin route). §4 builds it as
specified. §11.15a and §11.15b add the two things the brainstorm asked for that
§11.15 does not have: a way for the community to be asked, and a way for one
reader to get a body without changing what the instance holds for everyone.

### 4.1 §11.15 is built as specified, with one addition

The setting, the two values, the refusal paths, the per-source narrowing
override, the non-degradation rules, the four admin routes and
`instance_retention_policy` / `instance_retention_source_overrides` all stand
as written.

**Addition — an explicit refusal reason per path.** §11.15 already requires
that every refusal name the instance's policy. The addition is a **stable reason
code** per path (`RETENTION_AGGREGATE`, `RETENTION_AGGREGATE_SOURCE_OVERRIDE`,
`RETENTION_SOURCE_BLOCKED`, `RETENTION_VANISHED`) so that the six refusal paths
— URL import, file upload, clipboard paste, preservation batch, federated
announcement, cache fill — are enumerable and testable rather than six
hand-written strings that drift.

### 4.2 An aggregating instance has a preservation debt, and nothing surfaces it

§11.13 marks a vanished source unreachable. On an `aggregate` instance, a
vanished source with no cached body is a work that is **gone**, and the instance
has agreed in advance to let that happen to every work it aggregates.

Nothing counts those. An operator on `aggregate` sees a storage figure that
looks healthy and has no number for "works this instance is currently past
saving". One is added, on the operator dashboard:

```text
works_past_saving     aggregated works whose origin is unreachable and which
                      this instance holds no body for
```

It changes no behaviour and it is not public. It exists because the honest
number is usually what changes the operator's mind, and §11.15's design
decision is otherwise invisible to the person who made it.

---

## §11.15a Retention proposals — the community may be asked

**Addition.** The retention setting is the operator's, and the operator may ask
the community what they think before changing it. §19.15 governs the trust bar.

```text
retention_proposal_mode = off | advisory | binding     # default: advisory
```

- **`off`** — no proposals are accepted for retention.
- **`advisory`** (default) — a proposal records community preference and the
  operator responds in the modlog. The setting moves only through
  `PATCH /api/v1/admin/retention/policy`. The tally never writes the setting.
- **`binding`** — opt-in per instance. A proposal that reaches quorum sets the
  setting, subject to §5.3's asymmetry, cooling-off and operator override.

The precedent is §29.4: the roadmap board's voting trust gate is explicitly
configurable, and ADR 0023's division of authority is the same one this
extends — **Elo is the community's, stage is the operator's** — except that
`binding` lets a quorum of the community move a setting, so the analogy has a
limit and §5.3 is where the limit is drawn.

### 5.1 Proposals are proposals, not polls

A retention proposal is a structured motion — a proposed `body_mode`, the
source family it would apply to, a rationale, and a closing date — not a
one-line survey question. The distinction is the closing date. A poll answers
"what do you think today"; a proposal has a deadline and an outcome, and a
reader who arrives late can still see what happened to it, which is the
difference §29.5 already draws between a roadmap and a suggestion box.

### 5.2 The tally is a quality-weighted count, not a headcount

Per §29.6, **which reader voted is not a surface anywhere in the product**.
Individual ballots are never shown; the tally is an aggregate weighted by
§16.16's demand weight, so a week-old account counts for less (§19.2's safety
ramp). An unweighted headcount is not used: it is the shape of a popularity
contest, and §0.3's "no purchased ranking" is the reason the weight is there
rather than left off.

### 5.3 `binding` mode is bounded, asymmetric, and always overridable

A vote that sets a storage policy is not like a vote that sets a colour, and
the amendment says so rather than treating "constitutional" as costless.

- **It is opt-in per instance, and enabling it is itself a modlog entry.**
  `binding` is the operator saying in advance that the community may spend
  their storage.
- **Quorum is asymmetric by cost direction.** A proposal that **narrows**
  storage (any source family → `aggregate`) passes at the ordinary bar. A
  proposal that **widens** storage (`aggregate` → `cache`, or adding a source
  family to the cache set) passes at a **higher** bar, because it commits
  storage and bandwidth indefinitely. `retention_widen_quorum` is
  configuration, defaulting to the §19.4 "high-impact" bar of three.
- **A cooling-off period before it takes effect.** `retention_cooling_days`,
  default 14. The setting is committed at the end of it, not at quorum, and the
  pending change is visible on the operator dashboard for the whole period.
  The reason is that the cost is deferred and invisible: a reader votes today
  and the storage bill arrives next quarter.
- **The operator may always override, visibly.** A modlog entry with a reason
  sets the setting back. The override does not cancel the tally and the
  community can see that it happened. An operator who cannot overrule a vote is
  not an operator, and a system that pretends otherwise is lying about who
  holds the instance.
- **No vote may set anything that buys authority.** A retention proposal may
  change `body_mode` and its source overrides and nothing else. It may not
  reach trust levels, ranking signals, moderation queues, credit economics or
  eligibility (§0.3). This is enforced by the proposal's type carrying only
  retention fields, not by a check at the handler.

### 5.4 Storage budget framing, recorded as considered and not adopted

`cache` costs the operator storage and bandwidth forever. `aggregate` costs the
community preservation, invisibly and later. **A binary vote between them asks
the wrong question**, because the cost of one option is immediate and visible
to the voter and the cost of the other is deferred and paid by somebody else.
Most readers will pick the free-today option, and the result will be an
instance that has quietly stopped preserving.

A budget-allocation shape was considered: the operator sets a storage budget,
the community allocates it across the works people want kept, and the
allocation is what the operator applies. §16.16's demand weight is already a
reader's pull on unwritten content, so the mechanism is not foreign to this
spec. It is **not adopted here** for two reasons, both load-bearing: it makes a
reader's demand weight a storage *instruction*, which is a much larger
commitment than this amendment should make on its own; and it is only
meaningful once §4.1's retention setting is built and there is a real number to
allocate. It is recorded so a later draft extends it rather than re-derives it.

### 5.5 Acceptance

- A reader below the configured trust bar cannot open or file a retention
  proposal and is told the bar.
- In `advisory` mode a passed proposal does not change the setting; the
  operator's modlog response is required, and the setting only moves through
  the admin route.
- In `binding` mode a widening proposal at the ordinary quorum does **not**
  commit; at the widened quorum and after the cooling period it does.
- A pending change is visible to the operator for the whole cooling period.
- An operator override sets the setting back, is recorded with a reason, and
  leaves the tally intact and visible.
- No individual ballot is readable by any other reader through any route.
- A retention proposal payload carrying any field other than retention fields
  is refused by name.

---

## §11.15b A reader may ask for a copy; a reader may not be shown one

**Addition.** The request was: let readers above a trust level be *shown* the
body of a cached external work while lower-trust readers get metadata only, on
the same instance. The second half of that is refused, and the first half is
delivered in the form that holds.

### 6.1 The view is refused, with the spec's own precedent

§11.15 states the setting "applies to the instance, never to a request, a work,
an importer, an extension or a federated peer". A view that varies by the
viewer's trust level **is** a per-request override of the instance setting.
§19.1 states that "core publishing and reading remain available at TL0", and a
TL0 reader on such an instance could not read a work whose bytes are sitting
in the instance's storage.

The spec has already faced this exact shape and written down why the tempting
answer is wrong. §11.11, on preservation batches: "a batch that ran anyway as
metadata-only would be the same request answered two ways depending on who
asked." A trust-tiered body view is that fault, in the reading direction.

**The refusal, recorded so it is not re-proposed:** the instance's
`body_mode` does not vary by reader trust level, and no body is ever withheld
from a reader who is eligible to read it by being shown to a reader who is not.

### 6.2 The request is provided, and it gives the trusted reader the bytes

A reader at or above the configured trust level may **request that this
instance take a copy** of an external work body for them. This is §10.4.1's
existing durable per-reader snapshot, reached through a door.

- The request is a job, bounded by the source's robots posture and pacing
  (§11.5) exactly as any import is. It is not a synchronous fetch.
- It produces a **personal snapshot referenced by that reader's copy**, so the
  trusted reader reads, downloads and reads offline — the actual bytes, not a
  preview.
- It is recorded, with who asked and when, in the same audit surface as any
  other storage event.
- On a `cache` instance, every reader who is eligible to read the work may then
  read the snapshot through the normal read path, because §11.15's `cache`
  contract already makes a cached body available to readers of the work. The
  request removes the *storage* barrier for one work; it does not create a
  readers'-tier around it.
- On an `aggregate` instance it is **refused by name**, with
  `RETENTION_AGGREGATE`, exactly as §11.15 refuses every other body path.

This is the honest version of what was asked. The trusted reader ends up with
the work. What does not happen is an instance where two readers, looking at the
same work, are told different truths about whether it has text.

### 6.3 Why the ceiling is the configured trust level and not lower

`retention_body_request_min_trust` defaults to 2 and is **configurable**,
matching §29.4's precedent for a configurable voting bar and §0.3's rule that
the bars which may not move are the ones that grant authority (this one grants
storage, not authority). The floor is 0: a `cache` instance already serves
cached bodies to any eligible reader, and this setting only governs the extra
cost of a body nobody has asked for yet.

### 6.4 Acceptance

- A reader below the configured bar is refused with the bar stated, and no
  fetch is attempted.
- A request on a `cache` instance produces a per-reader snapshot and the bytes
  are readable, downloadable and offline-capable by that reader.
- A request on an `aggregate` instance is refused with `RETENTION_AGGREGATE`.
- No route, surface or rendered page varies in whether a body is shown
  according to the viewer's trust level, and a test fails the build if one
  appears.
- Every request is bounded by the source's robots posture and pacing, and a
  refusal to read is not retried.

---

## §19.15 Trust bar for retention proposals

**Addition.** §19.14 fixes the exchange's trust bars as consts because a
config key that lowered them would be purchased moderation authority. This
section states why a retention proposal's bar is a different kind of number.

- **A retention proposal requires the configured minimum trust level
  (`retention_proposal_min_trust`, default 2).** It is configurable for the
  same reason §29.4's is: the question being asked is preference-shaped
  ("should this instance hold on to external text?"), and the answer carries no
  authority over any other reader, any ranking, or any account.
- **What the vote can reach is bounded by type, not by check.** A proposal
  carries retention fields only. There is no configuration under which a vote
  reaches trust levels, ranking, moderation, credits or eligibility (§0.3).
- **A vote never grants the proposer anything personal.** No trust, no credits,
  no badge, no placement, and no notification beyond the outcome. A reader who
  opened a retention proposal is not made more visible for it, which is the
  thing that turns a preference poll into a status ladder.
- **One instance, one question at a time.** An instance with an open retention
  proposal does not accept a second on the same setting; a second question
  about the same knob is a way to shop for the answer that lands.

---

## §28 / §43 Checklist and configuration

New rows for §28.2 (available fiction) and §28.9 (foundational protections):

- [ ] A `robots_posture` of `strict` refuses every disallowed path; `metadata_only` reads metadata and stores no body; `permissive` is the previous `honour_robots = false`.
- [ ] `Crawl-delay` and the one-second floor are enforced identically under all three postures.
- [ ] A `metadata` fetch writes no revision, no FTS row, and no cache-fill job.
- [ ] Every overridden or discarded read is counted and the first per host is logged.
- [ ] `body_mode` has a setting, six refusal paths with stable reason codes, and the four admin routes.
- [ ] Retention is never varied per reader, and a reader above the configured bar may request a personal snapshot that is refused by name on an `aggregate` instance.
- [ ] A reader may not see another reader's retention ballot.
- [ ] A binding retention vote commits only through quorum, cooling period and recorded override, and reaches retention fields only.
- [ ] Preservation credits reverse when a destination dies, through a ledger entry and never a balance edit.
- [ ] No cumulative preservation score, XP, level or lifetime point total exists anywhere (§28.10).
- [ ] A crosspost of an imported work is refused by name rather than performed without a permission basis.
- [ ] `works_past_saving` is visible to the operator on an aggregating instance.

Configuration keys (§43, all with the §43 defaults table updated):

```text
[imports]   robots_posture              "strict"     strict | metadata_only | permissive
[imports]   robots_posture_overrides    {}           per-source-family narrowing, run-scoped
[imports]   honour_robots               (deprecated) compat read only; posture wins
[retention] proposal_mode               "advisory"  off | advisory | binding
[retention] proposal_min_trust          2           configurable (§19.15)
[retention] body_request_min_trust      2           configurable (§6.3)
[retention] widen_quorum                3           quorum for a storage-widening change
[retention] cooling_days                14          delay before a binding change commits
[preservation] threshold                3           distinct verified destinations
[preservation] decay_bp                 2500        marginal value per destination past threshold
[preservation] cap                      8           destinations that count at all
[preservation] recheck_interval_hours   168         destination availability re-check (§11.13)
```

Data model additions:

```text
preservation_destinations     instance config: archive name, base URL, match rule,
                              accepts_automated, enabled
preservation_targets          work_id, destination_id, state, verified_at, dead_at,
                              evidence_hash, created_by, created_at
preservation_credit_events    linked ledger entries per target; clawback is a
                              reversal with an idempotency_key naming the target
retention_proposals           proposed body_mode, source family, rationale,
                              opened_by, closes_at, tally, state, outcome
retention_proposal_votes      one ballot per account; never readable by another reader
retention_policy_changes      from, to, actor, reason, decided_at — the override record
```

---

## Explicitly Dismissed

| Proposal | Reason |
|---|---|
| Points, XP, or a cumulative preservation score | §28.10 refuses them by name; §9.7.1 refuses anything that compounds. Credits + badge carry the whole request. |
| Rewarding the crosspost *action* | Pays for link farms. Verification is checkable; a click is not. §2.1. |
| A trust-tiered *view* of cached bodies | §11.15's "never to a request"; §19.1's core reading at TL0; and §11.11's own precedent on answering one request two ways. §6.1. |
| A binary cache/aggregate poll presented as a fair choice | The costs are asymmetric and deferred; the cheap-today option wins by default. §5.4. |
| Storage-budget allocation by demand weight | Recorded as considered. Making a demand weight a storage instruction is a larger commitment, and §4.1 is unbuilt. §5.4. |
| Crossposting imported works to archives | Needs the §23.7 transfer manifest, which is deferred. Refused by name rather than performed without a permission basis. §2.7. |
| Choosing a `User-Agent` token to land in a more permissive group | Evasion by identity, not a posture. Refused in §1.5. |
| An override that outlives its import run | The §11.5 objection is to persistence, not granularity; the run-scoped form keeps the objection answered. §1.4. |
| A composite preservation score on the work page | §32.6. A dated fact is fine; a 0–100 score is a ranking. §2.4. |
