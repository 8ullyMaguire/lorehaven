# Lorehaven Spec Amendment — Trust-Gated Analytics

**Status:** Final plan
**Date:** 2026-09-26
**Source:** Consolidated analytics design (AI-generated, reviewed and restructured)
**Amends:** §9.6, §24.2, §24.3, §36.12
**Milestone:** M57

---

## Overview

Analytics are organised by **trust level**: how much a reader can see about
themselves, about their own works, and about the instance. TL0 gets a personal
dashboard. TL6 gets operational oversight. In between, the depth increases and
the privacy floor does not move.

The design arrived as a long enumeration of metrics. Restructured, it is three
things:

1. **A capability function.** `(scope, trust_level) -> permitted?` is the only
   gate in the system. Every route and every field consults it. A metric that
   is not a named capability is not renderable, so adding one to a dashboard is
   a code change with a name, not a template edit.
2. **A k-anonymity floor applied at the query, not at the display.** A count
   below the floor is not computed-and-hidden, it is never computed.
3. **An anti-list as executable tests.** The "never shown" list in the source
   document is a list of claims about behaviour. As tests, it is a list of
   facts that will fail the build when violated.

### What was kept, and what was cut

The source document enumerates roughly 300 metrics across seven trust levels.
Most are legitimate. The restructure keeps them and cuts four categories that
the document itself forbids elsewhere:

| Cut | Why |
|---|---|
| **A/B cohort assignment** (§24.6 says participants are not shown their variant) | Self-defeating. A dashboard that names your variant is a dashboard that changes behaviour. |
| **Signal weights applied to a user's actions** | §9.7.1 "Admin taste is invisible" and §0.3. Showing a reader the weight of their own bookmark tells them the taste model exists and what it values. |
| **Numeric taste resonance** | The document's own rule: labels only, own only. A number invites comparison, and a comparison is a leaderboard. |
| **All-time cumulative totals on any shared surface** | §9.7.1 "episodic, never cumulative" and "never all-time". Personal rolling windows are fine — they cannot be compared to anyone else's. |

Also dropped: the per-preset minimum-visibility table. Presets are retained as
an *upper bound* (see §2.4), because a preset that hides analytics is a
curation decision and a preset that reveals them is a privacy decision — and the
second is not one an operator should get to make by accident.

### Conflicts found against the existing spec

Four places where the source document contradicts the spec as written. Each is
resolved here in the spec's favour, and the reasoning is recorded so a later
reader does not "fix" it back.

1. **k = 5 versus k = 10.** The source document sets the floor at 5
   everywhere. §36.12 sets it at 10 for author dashboards. A reader moving from
   a public work page to their own author dashboard would find the threshold
   *fall*, which is the wrong direction. Resolved: the floor is 10 for
   anything about *other people*, 5 for anything about *yourself*.
2. **Geographic distribution.** §36.12 permits country-level geography for
   authors. The source document lists location among things "never shown at any
   trust level" except above a threshold. These can be reconciled: country
   aggregate, floor 10, no city or region, never for a work with a small
   readership. Retained from §36.12.
3. **"Never all-time" versus retention-cohort analysis.** The source document
   offers TL5 multi-year cohort retention. §9.7.1 forbids all-time cumulative
   displays that would function as a leaderboard. Resolved: cohort analysis
   compares *cohorts to each other* and never surfaces a single cohort's raw
   total, and no cohort metric is ever shown to an individual.
4. **Admin taste profile inspection.** The source document grants admin a panel
   showing dimensions, exemplars and anti-examples. §16.2 and §0.3 already
   permit this. Retained unchanged, with one addition: access is audit-logged,
   because §24.3 requires an applicable legal basis and §24.4 treats
   individual access as security telemetry.

---

## §1.0 Trust-Gated Analytics → §1.0 Analytics Model

**Modification.** Analytics become capability-gated. The gate is a single pure
function in the domain layer:

```text
analytics_allowed(scope, trust_level, instance_policy) -> permitted
```

`scope` is a **named capability** — a stable string, not a metric path. A
dashboard renders by iterating the capabilities its viewer is allowed, never by
iterating metrics and filtering them. The difference matters: iterating
capabilities means an unimplemented capability is invisible, while iterating
metrics means an unfiltered one leaks.

### 1.1 The capability set

Scopes, grouped. Names are stable identifiers; they appear in routes, in the
admin UI, and in the tests.

```text
own.reading.basic          TL0   words, chapters, streak, session length
own.reading.distribution   TL1   fandom/tag/mood distribution of own reads
own.reading.trend          TL1   reads per week over time
own.reading.reread         TL1   works read more than once
own.resonance.label        TL1   qualitative "Aligned: Strong/Moderate/…"
own.reading.percentile     TL2   own percentile against instance median
own.contribution.history   TL2   own imports, translations, fulfilled wishes

own.work.basic             TL0   reader count, word count, chapter count
own.work.reactions         TL0   quick-reaction counts, labels with n >= 5
own.work.retention         TL1   per-chapter drop-off curve
own.work.time_on_page      TL1   aggregate seconds per chapter
own.work.bookmark_timing   TL1   histogram of chapter number at bookmark
own.work.comment_timing    TL1   early-vs-late comment distribution
own.work.segments          TL2   "reached chapter X" counters
own.work.mood              TL2   dominant moods in comments and reactions
own.work.discoverability   TL2   search impressions, feed appearances
own.work.portfolio         TL2   this work vs. own other works
own.work.co_bookmark       TL3   "readers of this also enjoyed", n >= 10
own.work.long_retention    TL3   return-after-weeks percentage
own.work.series_cascade    TL3   series completion after part one

public.work.basic          TL0   word/chapter count, publication date
public.work.rating         TL0   mean and count, never a histogram (§9.4)
public.work.bookmarks      TL0   public bookmark count
public.work.reactions      TL0   public reaction aggregate if permitted
public.fandom.counts       TL0   works per fandom
public.instance.counts     TL0   total works, authors, translations (§24.2)

community.trending         TL1   which fandoms and tags are trending
community.bookmark_cf      TL1   "others also bookmarked" on own bookmarks
community.fandom_dash      TL2   per-fandom new works/day, readers/day
community.tag_cooccurrence TL2   which tags co-occur
community.search_trends    TL2   popular and zero-result queries
community.instance_activity TL2  instance words read today

community.fandom_growth    TL3   growth rate, author count, tag emergence
community.cross_fandom     TL3   reader overlap between fandoms
community.query_analysis   TL3   refinement and abandonment rates
community.collection_perf  TL3   engagement for collections you curate

steward.moderation_queue   TL4   case volume by category, resolution times
steward.positivity_perf    TL4   classifier precision/recall
steward.trust_distribution TL4   counts per trust level
steward.sanctions          TL4   sanction types and durations, aggregate
steward.duplication        TL4   duplicate-detection rates
steward.import_quality     TL4   success/failure by source and adapter
steward.discovery_perf     TL4   click-through by engine, diversity slots

strategy.cohorts           TL5   retention by signup cohort
strategy.economy_flows     TL5   credit flows per category, cap distribution
strategy.federation        TL5   inbound/outbound announcement volumes
strategy.translation_thru  TL5   request-to-delivery times

trustee.operations         TL6   health, storage, backup, migration state
trustee.financials         TL6   aggregate revenue, cost, payout
trustee.audit_trails       TL6   aggregate decision and quorum counts
trustee.breakglass         TL6   break-glass frequency and resolution

admin.taste_profile        role  §16.2 dimensions, exemplars, anti-examples
admin.moderation_audit     role  complete audit trail, self-access logged
admin.user_lookup          role  explicit lookup, audit-logged
```

**Acceptance.** Every entry in the table above has a name. Adding a metric means
adding a name here and implementing it, or it does not exist.

### 1.2 The never-shown list, as scope names

The anti-list is expressed as capabilities that **do not exist**, which is the
only form in which it can be enforced:

```text
ab.variant_assignment     never — §24.6
ab.signal_weights         never — §0.3, §9.7.1
ab.resonance_numeric      never — own only, label form is own.resonance.label
ab.pseud_linkage          never — §7.2
ab.other_reading_history  never — §9.5
ab.other_credit_balance   never
ab.other_earnings         never
ab.shadowban_state        never — §19.6
ab.vanguard_reason        never — §16.18
ab.session_identifiers    never — §24.3
ab.feature_flags          never
ab.individual_queries     never — §24.3 forbids retaining raw queries
ab.comment_scores         never — aggregate performance only
```

An implementation that references any of these names does not compile against
the registry, because the registry does not contain them. The tests below
assert the absence rather than trusting a comment.

---

## §2.0 Trust-Gated Analytics → §2.0 Privacy Floor

**Modification.** The floor is applied where the count is produced, not where
it is displayed. A suppressed value must never exist in a response, a log, or a
serialised aggregate — a display filter that runs after the query has already
fetched five individual rows has already had the privacy incident.

### 2.1 Two floors

```text
k_self   = 5    aggregates about the viewer
k_others = 10   aggregates about anyone else (§36.12)
```

`k_others` is the §36.12 value. A reader whose own dashboard shows a 6-person
breakdown must not then see an author dashboard with a 6-person breakdown of a
different set of people.

### 2.2 Coarsening, not suppression

Below the floor, a value is **coarsened** where a truthful coarse value exists
and **suppressed** where it does not.

```text
"A new work"                     instead of "1 work published Tuesday"
"fewer than 10 readers"          instead of "7 readers"
```

The coarse form must not be a numeric range a reader can narrow by asking
again. "Between 1 and 9" is a range; "fewer than 10" is a floor, and the floor
is the true statement.

### 2.3 Rounding

Timestamps in aggregate views round to the hour (daily views) or the day.
Precise per-reader timestamps are never aggregated into anything a second
reader can see.

### 2.4 Opt-out and instance policy

Two independent switches, and they are not the same thing:

- **Reader opt-out** (`analytics_contribute`): excludes that reader's actions
  from *all* aggregates they appear in, including their own dashboard's
  instance-relative figures. It does not disable their history, resume, or
  credits.
- **Instance preset** (`gallery`, `archive`, `commons`, `showcase`, `sandbox`):
  an upper bound on capabilities. `gallery` reduces the public surface to
  `public.*` and `own.reading.basic`; `sandbox` removes the upper bound. A
  preset can only *remove* capabilities, never add one above the trust level.

`showcase` is documented here as author-analytics-plus-public-counts rather than
as a separate visibility model, because the distinction from `archive` is a
matter of degree and a second near-identical code path would be one more thing
to keep correct.

### 2.5 Legal basis and retention

§24.3 already requires a documented legal basis, consent where required, and
estimation limits. This amendment adds the operational half:

- Raw reading events are retained as long as the reader keeps them in history
  (§9.6 controls that retention, not this section).
- Aggregates derived from opted-out readers are recomputed, not deleted — the
  derived table is rebuilt on the next batch.
- Aggregate tables carry a `computed_at` and a `source_window`, so a stale
  number is visibly stale rather than silently old.

---

## §3.0 Trust-Gated Analytics → §3.0 Metric Definitions

**Modification.** Every capability carries a **documented computation**, because
a number nobody can reproduce is a number nobody can trust. A metric ships with:

```text
definition     the formula, in words
sample_size    the denominator
freshness      the refresh interval (§4.2)
suppression    the floor and what happens below it
approximation  what is estimated, and by how much, or "exact"
```

**Acceptance.** A capability without all five is not renderable. The registry
rejects an incomplete definition at construction, so this cannot be skipped.

### 3.1 Definitions that need pinning down

The source document listed these without formulas. Each is decided here:

| Metric | Definition |
|---|---|
| Unique readers | distinct `pseud_id` with a reading event in the window. Not distinct sessions. |
| Completion rate | readers who reached the final chapter ÷ readers who reached the first. Both ends required, so a reader who opened the work page and left is not a denominator. |
| Drop-off | per chapter, `reached(chapter n) ÷ reached(chapter 1)`. A monotonic non-increasing series by construction. |
| Time on page | wall-clock between two progress updates on the same chapter, capped at 30 minutes per gap and summed. Estimated; the cap is the approximation and is disclosed. |
| Reading pace | summed word count ÷ summed reading time, over chapters finished. Not per-session, because a session that spans two chapters divides unevenly. |
| Trending | z-score against the trailing 30-day baseline for that fandom or tag, flagged above 2σ. A boolean, never a magnitude — a magnitude is a leaderboard. |
| Reaction count | distinct readers per label, floor 5. A reader who reacts 40 times counts once. |
| Bookmark timing | chapter number at first bookmark, bucketed, floor 5 per bucket. |
| Co-bookmark | pairs appearing in ≥10 readers' bookmark sets, top 10. Requires ≥10 readers on the work first. |
| Reader percentile | rank against the instance median for the same window, own only. |
| Taste resonance label | thresholds over the scalar projection: <0.4 Developing, <0.7 Moderate, else Strong. Never the score. |

---

## §4.0 Trust-Gated Analytics → §4.0 Access and Delivery

**Modification.** Delivery follows the existing token and rate-limit
architecture (§3.5, §23.1, §3.8) rather than introducing a parallel one.

### 4.1 Routes

```text
GET /api/v1/me/analytics                    own, all trust levels
GET /api/v1/me/analytics/{capability}       one capability, own
GET /api/v1/works/{id}/analytics            author only, own works
GET /api/v1/stats/{capability}              public or gated per registry
```

`/stats` is the §24.2 public statistics route, now registry-driven rather than
a fixed list, so a preset can narrow it without a code change per metric.

### 4.2 Freshness

```text
real-time      own history, own credit balance
minutes        own reading counts, reaction counts
hourly         work-level aggregates
daily          community dashboards, cohorts, tag co-occurrence
```

Every response carries `computed_at` and `source_window`. A client that caches
by `computed_at` shows "updated 4h ago" rather than a number that looks live.

### 4.3 Export

§24.7 own-data export covers TL0+. Analytics export for aggregates is separate
and gated: own-work aggregates at TL2, community aggregates at TL3, each
re-applying the floor at export time — a floor applied at query time does not
travel with the rows if the reader edits them locally.

---

## §5.0 Acceptance

- A viewer sees exactly the capabilities `analytics_allowed` permits, and the
  set is enumerable without reading any dashboard code.
- No aggregate about other people is returned below `k_others` = 10, and the
  check is at the query.
- A reader who has opted out is absent from every aggregate, and rebuilding the
  derived table reproduces that.
- An author sees their own work's analytics and cannot see another author's,
  including via a direct request.
- Every number carries a definition, a sample size, a freshness timestamp and
  a suppression rule.
- No surface renders a signal weight, a numeric resonance, a variant
  assignment, a shadowban state, or another reader's history — and a test fails
  the build if one appears.
- `gallery` hides every community capability; `sandbox` hides nothing; no
  preset can raise a trust level's ceiling.
- An instance with no readers shows "not enough data" everywhere, never zero.

**Non-goals.** A real-time analytics pipeline. Per-reader percentile
leaderboards. Any comparison between two individuals. Any surface that would
make raw reading volume a score.
