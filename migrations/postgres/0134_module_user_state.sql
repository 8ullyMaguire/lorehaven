-- 0134 — §59.6 per-user module state (M65-04).
--
-- Why a table and not a JSON blob on the account: §46's rule (per-domain tables,
-- never blobs) and the fact that the gate reads ONE of these rows on the way into
-- a surface. A blob would make that a parse on every request.
--
-- Why `enabled` when a row may only ever narrow: the row means "this reader has
-- an opinion", and the column records which opinion. Narrowing is enforced in the
-- writer (`is_off`/`set_off` in crates/db/src/module_state.rs) and in the route,
-- not by omitting a column that a later widening would then have to backfill.
--
-- Why no foreign key on account_id: §51 erases by pseudonymising the account row
-- rather than deleting it, so ON DELETE CASCADE would never fire and would imply
-- a guarantee this table does not get. The snapshot policy for this column is
-- `rekey_account` (docs/snapshot-column-policy.json) — a preference is not
-- content, so it is rekeyed with its account instead of dropped, and M45-51's
-- erasure pass owns that.
--
-- Erasure coverage is recorded as M65-10 so it is not a claim made only here.

CREATE TABLE IF NOT EXISTS module_user_state (
    account_id TEXT    NOT NULL,
    module_id  TEXT    NOT NULL,
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT    NOT NULL,
    PRIMARY KEY (account_id, module_id)
);

CREATE INDEX IF NOT EXISTS idx_module_user_state_module
    ON module_user_state (module_id);
