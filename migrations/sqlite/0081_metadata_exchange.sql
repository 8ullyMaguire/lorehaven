-- M57: the metadata exchange (spec §11.17, §15.17, §19.14, §16.16.1).
--
-- Both dialects: identical column sets, TEXT timestamps, INTEGER counters
-- (ADR 0004).
--
-- Four tables, and each one exists because a specific rule in the spec needs
-- somewhere to live that a nullable column pair cannot express — the same
-- reason `claims` needed a `status` column in 0080.
--
-- `exchange_signals` is the received evidence. It is never rewritten: §15.17
-- requires the originating signal row to be retained as provenance when an
-- entity is curated, and a row that is updated in place cannot be provenance.
-- The primary key is the content hash, which is also §11.17's deduplication
-- rule made structural rather than a procedural "SELECT then INSERT" that two
-- concurrent submitters can both pass.
--
-- `exchange_signal_entities` is the many-to-many from a signal to the names it
-- named. It is a join table rather than a JSON column because §15.17's
-- acceptance criteria are per-entity — "a signal naming a known tag adds an
-- alias rather than creating a duplicate" is a statement about one name, and it
-- cannot be asserted against an opaque blob without reparsing it in every query.
--
-- `canonical_entities` is the instance's own view of a name: whether it is
-- curated or an unverified candidate, and how many distinct signals have named
-- it. That count is review priority and nothing else — §15.17 and §16.16.1 are
-- explicit that it is never a demand weight, never a ranking input, and never a
-- count of readers, so the column is named `signal_count` and carries a comment
-- saying so, because a column called `reader_count` would be read as a headcount
-- by the next person who sees it.
--
-- It holds the unverified/curated split as its own `review_status` rather than
-- inferring it from a curator or a timestamp. §15.17's load-bearing claim is
-- that an unverified entity is usable immediately and *visibly* so, so the
-- distinction has to be a value a query can filter on, not an absence.
--
-- `exchange_settings` is the operator opt-in. §11.17 requires both sides to opt
-- in independently and says "no request shape turns on a server that did not
-- enable it", so the flag is a row rather than a config key: it must be
-- readable in the same transaction as the request without a config reload, and
-- an endpoint that appears only when a config file is edited is an endpoint whose
-- exposure nobody can audit at runtime.
--
-- The instance id and the rate-limit counters live here too. §11.17's limit is
-- per account and per source IP with a configured default, and the window start
-- is stored rather than derived from a count so that two concurrent batches
-- cannot both read a count below the limit and both write past it.

CREATE TABLE exchange_settings (
    id TEXT PRIMARY KEY,
    -- §11.17: the server opts in separately from each client. Absent row or
    -- `enabled = 0` means the exchange does not exist for this instance, and
    -- every route answers 404 rather than 403: a 403 confirms the door exists.
    enabled INTEGER NOT NULL DEFAULT 0,
    -- Submissions per account per hour (§11.17 default 1000). Configuration.
    rate_limit_per_hour INTEGER NOT NULL DEFAULT 1000,
    -- The account/submitter scope. Signals are retained, so who sent one is
    -- recorded — but the GET /canonical response never carries it (§11.17).
    instance_id TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE exchange_signals (
    -- The content hash of the signal body, and the primary key. §11.17: "a
    -- signal batch is deduplicated by content hash before it is stored, so a
    -- re-import costs a submitter nothing and creates no second record." Making
    -- it the key makes that true under concurrency, which a check-then-insert
    -- cannot be.
    content_hash TEXT PRIMARY KEY,
    work_id TEXT,
    account_id TEXT NOT NULL,
    source_instance TEXT,
    -- The full signal as received, verbatim. Retained so a curated canonical
    -- value can cite its provenance and so a disputed value can be shown the
    -- evidence rather than argued about.
    payload TEXT NOT NULL,
    -- §11.17: the submitter is recorded but never revealed. `GET /canonical`
    -- must not return who sent a signal, so this column is read by retention
    -- and rate-limit paths only.
    submitted_at TEXT NOT NULL
);
CREATE INDEX exchange_signals_work ON exchange_signals (work_id);
CREATE INDEX exchange_signals_account ON exchange_signals (account_id);
CREATE INDEX exchange_signals_submitted ON exchange_signals (submitted_at);

CREATE TABLE exchange_signal_entities (
    content_hash TEXT NOT NULL REFERENCES exchange_signals (content_hash) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    norm TEXT NOT NULL,
    PRIMARY KEY (content_hash, kind, norm)
);
CREATE INDEX exchange_signal_entities_norm ON exchange_signal_entities (kind, norm);

CREATE TABLE canonical_entities (
    kind TEXT NOT NULL,
    norm TEXT NOT NULL,
    -- The canonical form a curator set, or the first spelling seen while
    -- unverified. Always populated: §15.17 requires an unverified entity to be
    -- usable, and an entity with no canonical form has nothing to display.
    canonical TEXT NOT NULL,
    -- 'unverified' / 'curated' — the same two values as
    -- `taxonomy_nodes.review_status` (migration 0082), deliberately. Two tables
    -- describing one state must not spell it two ways, or a query joining them
    -- silently drops every curated row.
    --
    -- §15.17: 'unverified' is a usable state, not a pending one. A new name from
    -- a signal may be attached to a work, searched and browsed immediately; it
    -- is simply never presented as curated.
    review_status TEXT NOT NULL DEFAULT 'unverified',
    -- §15.17: review priority only. Never a demand weight (§16.16.1), never a
    -- ranking input, and NEVER a count of distinct accounts — one account
    -- signalling repeatedly is indistinguishable from many signalling once, so a
    -- reader headcount is not derivable from this and any surface implying one
    -- is wrong (§11.17).
    signal_count INTEGER NOT NULL DEFAULT 0,
    curated_by TEXT,
    curated_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (kind, norm)
);
CREATE INDEX canonical_entities_status ON canonical_entities (review_status);
-- The review queue's own ordering (§15.17: the name with forty distinct signals
-- is examined before the name with one).
CREATE INDEX canonical_entities_queue ON canonical_entities (review_status, signal_count DESC);

-- §16.16.1 latent demand: a signal for a work the instance does not hold creates
-- or reinforces a demand item, ONCE PER WORK PER SUBMITTING INSTANCE. The
-- primary key is what makes "once per work per instance" structural. There is no
-- column for a submitter count here either, for the same reason as above: it
-- could not be read as a headcount and is never shown as one.
CREATE TABLE exchange_latent_demand (
    work_id TEXT NOT NULL,
    source_instance TEXT NOT NULL,
    -- A demand item that has been seen once and one seen on forty sibling
    -- instances are the same demand, not forty votes (§16.16.1: "re-signalling
    -- the same work adds no weight").
    reinforced_at TEXT NOT NULL,
    signal_count INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    PRIMARY KEY (work_id, source_instance)
);
CREATE INDEX exchange_latent_demand_work ON exchange_latent_demand (work_id);
