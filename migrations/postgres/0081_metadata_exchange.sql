-- M57: the metadata exchange (spec §11.17, §15.17, §19.14, §16.16.1).
--
-- PostgreSQL arm of `migrations/sqlite/0081_metadata_exchange.sql`. Identical
-- column sets and column order; only the types and the boolean encoding differ
-- (ADR 0004). `INTEGER` for booleans becomes `BOOLEAN` here, and a bare
-- `DEFAULT 0` for a boolean column is a type error on this backend, so the
-- defaults are `false`/`true` to match.
--
-- The one place the arms are not textually identical is the queue index: SQLite
-- and PostgreSQL both accept `DESC` in an index definition, so the ordering
-- index is written the same way in both. It is created with an explicit `DESC`
-- because a plain ascending index cannot serve `ORDER BY signal_count DESC` —
-- PostgreSQL will scan it backwards, which is correct but not free, and the
-- review queue is the one query that runs on every curation page load.

CREATE TABLE exchange_settings (
    id TEXT PRIMARY KEY,
    -- §11.17: the server opts in separately from each client. Absent row or
    -- `enabled = false` means the exchange does not exist for this instance, and
    -- every route answers 404 rather than 403: a 403 confirms the door exists.
    enabled BOOLEAN NOT NULL DEFAULT false,
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
