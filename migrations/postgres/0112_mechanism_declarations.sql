-- M45-18: the faucet/sink declaration registry.
--
-- Identical to migrations/sqlite/0112_mechanism_declarations.sql. The two files exist
-- separately rather than being generated from one source because most migrations in this
-- codebase do differ -- `works.id` is TEXT here and uuid on the SQLite twin, for six
-- earlier tables -- but *this* table has no uuid and no JSON, so the only differences are
-- the explicit casts and `ON CONFLICT`. That is worth noticing: it is the first pair in a
-- while where the split buys nothing, and the comments below are duplicated rather than
-- shared only so that reading either file tells the whole story.
--
-- The reason this table is keyed on a mechanism name and not on
-- `credit_transactions.reference` is the one that is easy to get wrong: `preservation.rs`
-- posts two transactions sharing the same `reference` (a member id) on *opposite* sides of
-- the closed loop -- `Preservation` is a sink, `PreservationReclaim` is a faucet. A registry
-- keyed on the literal reference could not classify them, and since member ids differ per
-- member it would match none of them and report the entire preservation mechanism as
-- undeclared.

CREATE TABLE IF NOT EXISTS mechanism_declarations (
    mechanism_key  TEXT PRIMARY KEY,
    -- Same three values as `Flow` in crates/domain/src/flows.rs, so the store can hand this
    -- string to `Flow::parse` without a second mapping to keep in sync.
    flow           TEXT NOT NULL CHECK (flow IN ('faucet', 'sink', 'neutral')),
    label          TEXT NOT NULL,
    created_at     TEXT NOT NULL
);

-- The four mechanisms the economy actually posts today, read off the four
-- `post_transaction` call sites rather than guessed.
INSERT INTO mechanism_declarations (mechanism_key, flow, label, created_at)
VALUES
    ('tips',                 'sink',   'Reader tips',          '2026-10-02T00:00:00Z'::timestamptz),
    ('preservation_dues',    'sink',   'Preservation dues',    '2026-10-02T00:00:00Z'::timestamptz),
    ('preservation_reclaim', 'faucet', 'Preservation reclaim', '2026-10-02T00:00:00Z'::timestamptz),
    ('author_earnings',      'faucet', 'Author earnings',      '2026-10-02T00:00:00Z'::timestamptz)
ON CONFLICT (mechanism_key) DO NOTHING;
