-- M52-05: Curator flag for accounts (recommendation: curator prior strategy).
ALTER TABLE accounts ADD COLUMN is_curator BOOLEAN NOT NULL DEFAULT FALSE;
CREATE INDEX accounts_is_curator ON accounts (is_curator);
