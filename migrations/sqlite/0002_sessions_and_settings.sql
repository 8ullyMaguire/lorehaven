-- Migration 0002 — sessions gain an active pseud; content settings gain a home.
--
-- Dialect: SQLite.
--
-- Two additions for Milestone 2 (spec §7):
--
--  1. `sessions.active_pseud_id`. The active pseud is *session-scoped*, not
--     account-scoped: a person legitimately signs in as a different face on
--     their laptop and their phone. Storing it on the account would make one
--     device silently change what another device is posting as.
--
--  2. `content_settings`. Spec §7 requires `GET/PATCH /settings/content`
--     distinct from `GET/PATCH /settings/privacy`. Privacy is "who may see
--     what of mine"; content is "what do I want to be shown". They have
--     different defaults and different lifecycles, so they are different
--     tables rather than more keys in `privacy_settings`.
--
-- Deletion and retention: both additions cascade with their owning account.
-- `active_pseud_id` is `ON DELETE SET NULL` so deleting a pseud does not
-- destroy the session that was using it — the session simply falls back to the
-- account's first remaining pseud.

ALTER TABLE sessions ADD COLUMN active_pseud_id TEXT REFERENCES pseuds (id) ON DELETE SET NULL;

CREATE INDEX sessions_active_pseud ON sessions (active_pseud_id);

CREATE TABLE content_settings (
    account_id        TEXT    PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    -- The highest rating this account wants surfaced to it. Distinct from the
    -- *policy* ceiling in lorehaven-domain, which is what the operator allows;
    -- this is what the reader prefers within that.
    max_rating        TEXT    NOT NULL DEFAULT 'general',
    -- JSON array of warning strings the reader never wants to see.
    excluded_warnings TEXT    NOT NULL DEFAULT '[]',
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);
