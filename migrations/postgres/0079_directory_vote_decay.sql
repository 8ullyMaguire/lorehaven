-- M58 — Vote Decay (spec §39, amendment §2).
--
-- A rename only. No data movement, no new columns, no new index.
--
-- The column used to be `weight` and its comment claimed it was "recomputed
-- on trust/taste change". Under vote decay that is actively wrong: a weight
-- that is recomputed on trust change cannot also be a function of vote age,
-- and the two are now genuinely different things. `base_weight` is the
-- trust-and-taste weight at the moment of voting; the contribution to the
-- score is base_weight * decay(age), computed at read time.
--
-- A column whose name does not say which of the three meanings it carries is
-- a bug waiting for a reader who assumed a different one.
--
-- Byte-identical to the SQLite twin: the rename carries no type, so the
-- dialect convention (mirror SQLite types exactly) has nothing to diverge on.

ALTER TABLE directory_votes RENAME COLUMN weight TO base_weight;
