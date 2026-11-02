# M45-27 — exchanges, prompt memes and big bangs: the pairing, and nothing else

**Status:** planned 2026-11-02. Spec §18 (events), new §18.11. Migration `0136`.

## Premise, measured before planning

`docs/requirements.csv` M45-27 reads *"Exchanges, prompt memes, big bangs with matching engine —
Yuletide-style gift exchanges; anonymous prompt/fill boards; social, not transactional."* Two of
those three clauses are already built, and the third has no schema at all.

| what the row asks for | what exists | verdict |
|---|---|---|
| anonymous prompt/fill board | `requests` (`requester`, `prompt`, `anonym_until`) + `claims` (`claimant`, `fulfilled_by_work`) + `domain::events::request_is_anonymous` | **built** — a second board would be a second opinion about the same act |
| a place to hold it | `events` (name, document, created_by) + `event_participation` | built, and it is a **document**, not a programme |
| gift exchange / big bang pairing | **nothing** — no assignment table, no matcher, no reveal | **this ticket** |

Live instance, 2026-11-02: `events` **0**, `event_participation` **0**, `requests` **0**,
`claims` **0** (database `darkhold`, socket `/run/postgresql`). The whole §18 area is built and
unused, so every claim below is proven by tests and by a hand-run through the doors, not by traffic.

Schema as it stands (`migrations/postgres/0014_events.sql`):

```sql
CREATE TABLE events (id TEXT PRIMARY KEY, name TEXT NOT NULL, document TEXT NOT NULL,
                     created_by TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE event_participation (event_id TEXT NOT NULL, account TEXT NOT NULL,
                                  joined_at TEXT NOT NULL, PRIMARY KEY (event_id, account));
```

`created_by` and `event_participation.account` are **accounts**, so the matcher pairs accounts. The
gift is authored under a pseud and the work already carries its author; the exchange's secret is
*who is writing for whom*, which is an account-level fact until reveal.

## What is missing, and the shape it takes

A Yuletide-style exchange needs three things the document cannot hold:

1. **terms** — what a participant offers, what they want, and whom they refuse;
2. **a pairing** — one giver per recipient, nobody for themselves, exclusions honoured;
3. **a reveal** — the pairing is secret until a timestamp.

Two tables, one of them a small kind-tagged table rather than three near-identical ones:

```sql
CREATE TABLE exchange_terms (
    event_id TEXT NOT NULL,
    account  TEXT NOT NULL,
    kind     TEXT NOT NULL,   -- 'offers_fandom' | 'wants_fandom' | 'excludes_pseud'
    value    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (event_id, account, kind, value)
);
CREATE INDEX exchange_terms_lookup ON exchange_terms (event_id, kind, value);

CREATE TABLE exchange_assignments (
    event_id         TEXT NOT NULL,
    giver            TEXT NOT NULL,
    recipient        TEXT NOT NULL,
    assigned_at      TEXT NOT NULL,
    revealed_at      TEXT,
    matched_by       TEXT NOT NULL,   -- 'greedy' | 'augment'; the algorithm that produced it
    PRIMARY KEY (event_id, giver)
);
CREATE UNIQUE INDEX exchange_assignments_recipient ON exchange_assignments (event_id, recipient);
```

Three decisions worth stating:

- **`kind` rather than three tables.** The three kinds share their entire shape (event, account,
  value, time) and differ by one word. `work_character_roles`-style separation would triple the
  store for no query it makes easier; one CHECK keeps the vocabulary closed.
- **No foreign keys**, for the `supply_routing` reason (§58.3): §51 erases by rekeying an account
  rather than deleting it, so `ON DELETE CASCADE` would never fire, and a FK would make an
  erasure fail instead of rekey. `exchange_terms.account` and both assignment columns are declared
  `rekey_account` in `docs/snapshot-column-policy.json`.
- **The unique index on `recipient` is the invariant, not a convention.** "One giver per recipient"
  is enforced by the database, so a matcher bug cannot ship a double-gift; the test that tries to
  insert two givers for one recipient must fail on the constraint, not on an assertion.

## The matcher (`crates/domain/src/exchanges.rs`, pure, no I/O)

Input: participants with `offers: BTreeSet<String>`, `wants: BTreeSet<String>`,
`excludes: BTreeSet<String>`. Output: `Ok(Vec<Pair>)` or `Err(MatchFailure::Impossible { names })`.

- **Hard constraints.** No self-pairing; an edge is absent when either side excludes the other or
  when the two are the same participant.
- **Maximum cardinality first, weight second.** Cardinality via Kuhn's augmenting paths over the
  allowed-edge graph — so the pairing is maximum, and "3 of 7 matched" is never reported as success.
  Then improve total weight by deterministic 2-opt swaps, and record `matched_by` accordingly.
- **Weight** = `3·|offers ∩ wants|` (both directions count: gives what they want, and wants what
  they give) `+ 1` if the pair shares any fandom at all. Integer only, no floats, so two runs and
  two machines agree.
- **Determinism.** Participants are processed in `account` order and neighbours in `account` order;
  ties therefore resolve by id, not by hashmap iteration. The test asserts two runs over shuffled
  input produce byte-identical pairings.
- **Refusal names people.** When a maximum matching leaves anyone unmatched, the result is
  `Impossible { names }` — the surface answers `422` naming every unmatched account and writes
  **nothing**. A partial pairing is the one outcome a gift exchange cannot recover from: somebody
  is promised a gift and no amount of later matching changes that.

## Doors (`crates/app/src/routes/exchanges.rs`, module `exchanges`)

| method + path | who | answers |
|---|---|---|
| `POST /api/v1/exchanges/{event}/join` | session, event participant | `201` with the participant's terms; idempotent |
| `PUT /api/v1/exchanges/{event}/terms` | session, event participant | `200`; replaces that participant's terms wholesale |
| `POST /api/v1/exchanges/{event}/match` | the event's `created_by` | `201` with the pairings (as the operator sees them) or `422` naming the unmatched |
| `GET /api/v1/exchanges/{event}/mine` | session, event participant | `200` with `{recipient: null, revealed: false}` until reveal, then the recipient |
| `POST /api/v1/exchanges/{event}/reveal` | the event's `created_by` | `200`, sets `revealed_at`; second call reports the first timestamp rather than an error |
| `GET /api/v1/exchanges/{event}` | anyone | `200` with counts only: participants, matched, revealed |

- **`mine` is the anonymity rule.** Before reveal it answers the participant's own terms and
  `recipient: null` — not `403`, because the answer is a fact: you have not been told yet.
  `domain::exchanges::may_see_recipient(revealed, caller_is_giver)` carries the rule and is unit
  tested; the route may not re-derive it.
- **Matching is a write with a precondition.** It refuses when nobody has joined, when it has
  already run (`409`, naming `assigned_at`), and when the pairing is impossible (`422`, naming the
  accounts). It never overwrites a previous run — the record is the row, as in §58.3.
- **Terms are wholesale, not incremental.** `PUT` replaces the set, because a person editing
  "fandoms I want" is expressing a preference, not appending to one; an incremental API makes
  "remove a fandom" a second verb.

Module row: id `exchanges`, `default_on: true`, `relevance: 6`, `Toggle::Operator`, spec §18.11,
`source: crates/app/src/routes/exchanges.rs`, no nav (a seasonal surface does not belong in the
sidebar; the event page links to it). Relevance 6 rather than 10 because an instance that never
holds an exchange gives the surface nothing to do, and rather than 4 because the routes are inert
without an event — nothing is switched off by leaving it on.

## Verification, step by step

| # | command | expected |
|---|---|---|
| 1 | `cargo test -p darkhold-domain --lib exchanges` | 6 matcher tests pass |
| 2 | `cargo test -p darkhold-db --test exchanges` (SQLite) | 9 store tests pass |
| 3 | `DARKHOLD_TEST_PG_URL=… cargo test -p darkhold-db --test exchanges` | the same 9 pass on PostgreSQL |
| 4 | `cargo test -p darkhold-app --test m45_27_exchanges` | 8 route tests pass |
| 5 | mutation suite (below) | every mutation caught |
| 6 | `cargo clippy -p darkhold-domain -p darkhold-db -p darkhold-app --all-targets` | no new warning |
| 7 | `python3 scripts/ideas_catalogue.py` | `178 ideas`, 0 homeless |
| 8 | deploy + `curl -s http://127.0.0.1:8081/api/v1/meta` | build names the commit |

Mutations that must be caught: drop no-self edge; drop exclusion edge; return the greedy pairing
when augmenting fails; remove the weight term; sort neighbours by hashmap iteration; allow a
partial pairing to be reported as success; `may_see_recipient` true before reveal; drop
`matched_by`.

## Deferred, with reasons

- **Deadline reminders** (§18 events carry a document, not dates) — needs a notification job and a
  scheduler decision that is not this ticket's; the exchange works with an operator-run reveal.
- **A prompt-meme feed** — `requests` already serves it through the `events` module; a second feed
  would duplicate the surface M45-27's first clause already satisfies.
- **Re-matching an individual pair** — an operator can re-run only before assignments exist
  (`409` after), which is the honest rule: the promise is a row.
