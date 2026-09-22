-- 0059_vanguard_roles: vanguard role assignments and pins (spec §16.18, M18 Phase 4.2).
--
-- Dialect: PostgreSQL.
--
-- The vanguard_roles table tracks which accounts hold the Taste Vanguard
-- role, how they were selected, and when it expires. The vanguard_pins
-- table tracks works pinned by vanguards to fandom pages / the Vanguard
-- Picks shelf.

CREATE TABLE IF NOT EXISTS vanguard_roles (
    account_id  UUID PRIMARY KEY,
    granted_at  TIMESTAMPTZ NOT NULL,
    method      TEXT NOT NULL DEFAULT 'admin_appointment',
    expires_at  TIMESTAMPTZ,
    granted_by  UUID
);

CREATE INDEX IF NOT EXISTS idx_vanguard_roles_expires_at ON vanguard_roles(expires_at);

CREATE TABLE IF NOT EXISTS vanguard_pins (
    id          UUID PRIMARY KEY,
    account_id  UUID NOT NULL,
    work_id     UUID NOT NULL,
    pin_reason  TEXT NOT NULL DEFAULT 'curated',
    message     TEXT,
    pinned_at   TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ,
    deleted_at  TIMESTAMPTZ,
    FOREIGN KEY (account_id) REFERENCES accounts(id),
    FOREIGN KEY (work_id) REFERENCES works(id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_vanguard_pins_account_work ON vanguard_pins(account_id, work_id);
CREATE INDEX IF NOT EXISTS idx_vanguard_pins_expires_at ON vanguard_pins(expires_at);
