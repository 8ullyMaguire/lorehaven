# 14. Governance — guests, trust, jury

**Status:** reserved
**Milestone:** M3, M20, M21
**Source:** the corresponding section of the Darkhold specification

The estate standard GS-1…GS-5, in the order it prescribes. **GS-3 first**, then GS-1, then GS-2,
then GS-4, then GS-5.

## 14.1 GS-1 — guests are real identities

A guest is **a row in `accounts`** with an empty credential and `is_guest = true`. Not a shadow
table, not a cookie-only identity.

**Privilege parity:** a guest has the same functionality as a recently-registered account — level 1,
never a moderation level. A guest can post, comment, vote and report. A guest can never moderate,
and **a guest can never extend trust**.

**The door closes on MAU.** MAU = distinct *accounts* with at least one recorded action in the last
30 days. When MAU exceeds `guest.mau_limit` (default **500**), new guest identities are refused and
guest writes stop. Existing guest data is preserved and remains foldable.

| key | default |
|---|---|
| `guest.enabled` | true |
| `guest.mau_limit` | 500 (`0` disables guest access entirely) |
| `guest.mau_window_days` | 30 |
| `guest.on_cap` | `close` (or `throttle`) |
| `guest.grace_days` | 7 |
| `guest.parity_level` | 1 |

**Per-site firewalls, as configuration:** a guest has **no ActivityPub actor**; guest **uploads are
quarantined** and capped per day; on an adult-content instance, consent is **not** an age gate; and
where the instance's value is a member list, guests get the *functionality* but not the *directory*.

**Anti-abuse:** a guest's rate limits are a logged-in new user's — never higher, never lower. No
email recovery. One identity per cookie+IP fingerprint.

## 14.2 GS-2 — the fold

Every row a guest authored is re-parented to the new account **in one transaction**, then the guest
identity is tombstoned.

- **Schema-driven completeness.** The fold walks the schema for every column that references the
  identity and **fails when a column exists that the fold does not handle**. "A fold that silently
  misses a column produces 'my votes disappeared' months later; a hand-kept list of tables drifts
  the first time a migration lands. The check *is* the feature."
- **Idempotent and one-shot.**
- **Folding must not launder trust.** The fold grants **tenure** and **never score**: no
  reputation, no votes-received, no report-accuracy credit. Otherwise the guest door is a trust
  farm.

| key | default |
|---|---|
| `merge.enabled` | true |
| `merge.grants_trust` | **false** |
| `merge.grants_tenure` | true |
| `merge.notify` | true |
| `merge.tombstone` | `anonymize` (or `delete`) |

## 14.3 GS-3 — consent, and why it gates GS-1

The guest identity **is** a cookie, so consent is not a decoration: it is the gate that creates the
identity. **Until it is accepted, the site is anonymous read-only.**

- **Server-authoritative.** The toast is a courtesy; the refusal is the API's. A middleware refuses
  **mutating** requests without consent (403 with `{"consent":"required"}`); every read route stays
  open.
- **Stored as a cookie**, not localStorage.
- **One client wrapper** for the early refusal.
- **Refusal is real.** Refusing leaves the site fully readable and stores nothing persistent.
- **Versioned.** `consent.version` is stamped on the record; bumping it re-asks.

| key | default |
|---|---|
| `consent.required_for_guest` | true |
| `consent.version` | 1 |
| `consent.categories` | `essential` always on; `analytics`, `personalisation` opt-in |
| `consent.age_gate` | `required` (D-9 — the instance hosts adult content) |
| `consent.reask_on_version_change` | true |

## 14.4 GS-4 — trust, earned and elastic

**One ladder. 0 → 6.** Names and limits are configuration.

| level | name | typical requirement | unlocks |
|---|---|---|---|
| 0 | anonymous | — | read, search, public catalogue, the house recommendation stream (T0) |
| 1 | new | an account, or a guest | post, comment, vote, report (weight 1), the personal stream (T1) |
| 2 | member | tenure + accepted contributions + report accuracy | publish to shared galleries, merge proposals, tag proposals, **create a mood** |
| 3 | trusted | more of the above, weighted | curator tools: propose merges, vote on content, set topic tags, flag weight 2, **create a channel** |
| 4 | moderator | reviewed conduct | the moderation queue, fast-hide, lock/pin, flag weight 3, **the full engine (T2) if `full_min_trust` is 4** |
| 5 | senior reviewer | staff-designated | resolve reports, scoped bans, jury service, flag weight 5 |
| 6 | trustee | operator-designated | everything except instance-wide settings |

**Two-sided by design.** The *earned* side is tenure days, accepted contributions, vote score and
report accuracy, weighted per site in `trust.axes`. The *required* side is the operator's
requirement:

**Elastic allocation.** When a privilege's eligible pool is **empty**, the requirement falls until
at least `trust.elastic_min_holders` holders exist — but never below `trust.floor_level`. A site
with three users still has a moderator; a site with three thousand does not hand the power out.
Elastic grants are re-evaluated after `trust.elastic_decay_days`, and the requirement returns to
nominal as soon as enough people clear the real bar. **No ratchet.**

**Trust is earned through reviewed conduct** — not post count, not word count, not kudos, not
credits, not any activity-volume metric. The only automatic *demotion* is spam: three confirmed
spam reports in 90 days drops one level, never below level 2, with diminishing recovery
(`0.7^loss_count`).

### What the implementation settled

- **The ladder is configuration, and no route holds a level number.** A route asks for the
  *privilege* it needs — `ModerationQueue`, `ScopedBans`, `CreateMood` — and the ladder says
  which level holds it. The names, the unlocks and the levels are one configuration table, so an
  instance that wants its moderators at level 2 writes a different ladder rather than changing
  the code. The privilege vocabulary is **closed**: a configuration naming a privilege the build
  does not know is a startup error, because a privilege nobody holds is a privilege that
  silently unlocks nothing.
- **The ladder is cumulative.** A level holds everything the levels below it hold, so level 1
  does not repeat level 0's reading privileges. The specification's table lists only what each
  level *adds*, and the cumulative rule is what makes it mean what it says.
- **Level 0 is the anonymous reader and level 1 is "an account, or a guest."** A registered
  account is at level 1 from the moment it exists, and its level is *computed* rather than
  inferred from the absence of a row. The levels above it are unchanged, so this renumbers
  only the bottom of the ladder — and it is the reason "level 1 to submit" and "level 3 plus
  quorum to curate" (§15.2) mean what they say.
- **The full engine follows the tier threshold.** Level 4's row says "the full engine (T2)
  **if `full_min_trust` is 4**" — so `tiers.full_min_trust` decides where the full engine
  sits, and the ladder's row follows it. There is no second configuration key to keep in
  step, and no way for them to disagree.
- **The four axes each name their review**, and that is what makes them *earned* rather than
  *active*: tenure is time; contributions are proposals **passed** or **approved**, so somebody
  else accepted them; the vote score is the **public** ratings on the account's published work
  (a reader's private rating is their own business, and letting it move a writer's standing
  would make the axis a weapon); and the report accuracy is reports **upheld** minus reports
  **dismissed** — net, not a rate, because a rate rewards a reader with one lucky report as much
  as a reader with fifty confirmed ones.
- **The activity volume is measured and then thrown away.** It is counted so that the
  counter-example can be stated rather than asserted: ten thousand posts is a lower level than
  twenty accepted contributions. The score function does not read it, and the stored basis does
  not carry it.
- **A level is written with the basis that explains it**, and **every change writes an event**.
  The event log is **append-only, and enforced**: triggers refuse an update and a delete on both
  engines, because a trust history that can be rewritten is not a history. The cause vocabulary
  is closed in the code *and* in the database, because two lists that can disagree would be a
  list the database accepts and the code cannot explain.
- **The elastic requirement is stored, not computed per request.** The pool is a count over
  every account, and a trust gate is asked on every request a reader makes that they are not
  allowed to make. Storing it is also what makes **no ratchet** true by construction: the
  requirement is a pure function of the pool and has no memory of what it decided last time.
- **The trust refusal names both levels** — the one found and the one required — because a
  refusal that names neither is not actionable, and §0.3 makes the thresholds visible by design.
- **A guest never extends its trust**, and that is enforced rather than merely configured: a
  guest with ninety days of tenure would otherwise reach level 2 on the tenure axis alone, and
  the guest door is otherwise a trust farm.
- **The demotion's floor is not configurable.** A demotion that could take an account to level 0
  would make the account a guest again, and a guest's contents are foldable away. And
  *confirmed* means a reviewer **upheld** the report and the report said **spam**: an open
  report is an allegation, and a demotion is not founded on an allegation.

| key | default |
|---|---|
| `trust.levels[]` | per site |
| `trust.axes` | per site |
| `trust.require.<privilege>` | per privilege |
| `trust.elastic_min_holders` | 1 |
| `trust.floor_level` | 2 |
| `trust.elastic_decay_days` | 30 |
| `trust.guest_level` | 1 |
| `trust.guest_earns` | **false, always** |

## 14.5 GS-5 — self-governance

1. **Any user flags.** No moderator or admin needed to raise it.
2. **The community's own moderators** see it — not the instance operator. "The queue is
   admin-only" is the bug, not the design.
3. **A jury** is drawn **randomly** from the eligible pool (level ≥ `governance.jury_min_level`,
   default 3), size `governance.jury_size` (default 5), excluding the reporter, the reported, and
   anyone who has taken a side. **Recusal is enforced by the draw, not by convention.**
4. **Appeals** are decided one level up by a **fresh** jury — never the same people.
5. **The operator is emergency only.** Every use is logged and visible to users.

**Elasticity applies here too.** If nobody has reached level 3, the jury is drawn from the **top of
the actual ladder** with `governance.jury_elastic_min` (default 3) members.

**Outcomes are tiered and expiring:** warning → content removal → temporary restriction → ban,
with `governance.ban_review_days` (default 90) and an appeal always available.

**The firewall:** no governance decision may be made by a party to it, and **no engine or model is
the decider**. Safety decisions are never modulated; the human decides.

| key | default |
|---|---|
| `governance.enabled` | true |
| `governance.jury_size` | 5 |
| `governance.jury_min_level` | 3 |
| `governance.jury_elastic_min` | 3 |
| `governance.appeal_fresh_jury` | true |
| `governance.admin_emergency_only` | true |
| `governance.ban_review_days` | 90 |

## 14.6 The modlog

Every moderator, curator and operator action is recorded and **readable by any signed-in reader**,
with actor, action, target, reason and timestamp. Metamoderation votes are anonymised.

### What the implementation settled

- **Anonymous access is 401, not 403.** "403 answers 'yes, and you may not'" — but the modlog is
  not a door that exists only for the permitted. It is a public record that any reader of the
  instance may read, and being anonymous is a fact about the *reader* rather than about the
  record. A 403 would tell an anonymous caller that the modlog exists and that they may not see
  it, which is an answer to a question this section says they may ask.
- **The acting account is never published.** The audit log's `actor` column holds the account —
  right for an audit, which is the instance's own record — but the modlog is a *public* surface,
  and publishing an account id there would publish the very link §7 says no public surface may
  publish. **The actor is the acting pseudonym**, and a target that is an account is published as
  that account's public face.
- **The reason is a column, not something dug out of a blob.** The action's detail is a JSON
  document whose shape differs per action; a reason read out of it would be present in some
  entries and absent in others, and the contract says *every* entry carries one.
- **A panel decision is the instance acting.** An action with no pseudonym behind it is rendered
  as such — which is what an anonymised metamoderation vote is, since "the actor is the jury
  panel, not the voter".
- **The cursor is one implementation.** A cursor is a promise, and the promise has to be the same
  promise everywhere: the same encoding, the same `(timestamp, id)` pair, and a cursor this
  collection did not issue is a **validation failure rather than an empty page** — an empty page
  would say "you have reached the end" when the truth is "I did not understand you".

## 14.7 Quorum and curation

Where curation is consequential — a high-impact identity merge, a permanent ban, a taxonomy change,
a metadata correction, an extension approval, **the creation of a public channel** — a **quorum** of
human reviewers is required. Defaults are configuration: routine tag change 2, high-impact merge 3,
permanent ban 3, emergency containment 1 then review, metadata correction 2, extension approval 3,
**channel publication 2**.

**"Quorum" means human reviewers.** Three automated signals agreeing is a *confidence* threshold,
and calling it quorum would corrupt a load-bearing term.

## 14.8 Appeals

An appeal is filed against a sanction, decided by a fresh jury. The outcome ladder is explicit and
every expiry is automatic.

---
