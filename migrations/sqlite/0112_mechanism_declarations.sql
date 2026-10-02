-- M45-18: the faucet/sink declaration registry.
--
-- Every credit movement in the economy is posted with a `reference` and a `TxnType`, and
-- §53 asks for a dashboard that says which side of the closed loop each movement is on. The
-- obvious design -- key this table on `credit_transactions.reference` -- does not work, and
-- the reason is worth recording because it is not obvious:
--
-- `preservation.rs` posts two transactions whose `reference` is the *same member id*, one
-- a `Preservation` (a sink: the member paid dues) and one a `PreservationReclaim` (a
-- faucet: the dues came back). They are on opposite sides of the loop and share a key.
-- Because a member id differs per member, a registry keyed on the literal reference would
-- match none of them and report the whole preservation mechanism as undeclared.
--
-- So this table is keyed on a *mechanism name* -- `tips`, `preservation_dues`,
-- `preservation_reclaim`, `author_earnings` -- which the store derives from the transaction
-- type rather than from the reference. That makes the classification data instead of code:
-- a fifth mechanism is a row here plus a derivation clause, not a schema change.
--
-- `declaration` is the reason a mechanism can be *absent* rather than merely unlisted. The
-- dashboard's `undeclared` count is load-bearing: a mechanism nobody classified still moves
-- real credits, and omitting it would report a smaller economy than exists. Counting it is
-- the honest rendering; silently dropping it would be the inflation problem of §0.3 wearing
-- a dashboard.

CREATE TABLE IF NOT EXISTS mechanism_declarations (
    mechanism_key  TEXT PRIMARY KEY,
    -- 'faucet' pays into the loop (a reader earns, or gets credits back); 'sink' drains it
    -- (a reader spends); 'neutral' moves credits without changing the total. Deliberately
    -- the same three values as `Flow` in crates/domain/src/flows.rs, so the store can hand
    -- the string straight to `Flow::parse` without a second mapping to keep in sync.
    flow           TEXT NOT NULL CHECK (flow IN ('faucet', 'sink', 'neutral')),
    label          TEXT NOT NULL,
    created_at     TEXT NOT NULL
);

-- The four mechanisms the economy actually posts today. Read off the four
-- `post_transaction` call sites rather than guessed; if a fifth appears, this list is
-- incomplete and the dashboard will say so rather than quietly mis-report.
INSERT INTO mechanism_declarations (mechanism_key, flow, label, created_at)
VALUES
    ('tips',                 'sink',   'Reader tips',                     '2026-10-02T00:00:00Z'),
    ('preservation_dues',    'sink',   'Preservation dues',               '2026-10-02T00:00:00Z'),
    ('preservation_reclaim', 'faucet', 'Preservation reclaim',            '2026-10-02T00:00:00Z'),
    ('author_earnings',      'faucet', 'Author earnings',                 '2026-10-02T00:00:00Z')
ON CONFLICT (mechanism_key) DO NOTHING;
