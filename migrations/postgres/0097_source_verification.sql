-- Where THIS BUILD's host could not reach a source (M53-04).
--
-- spec §11.8 defines health as a claim about the SOURCE — and warns explicitly
-- against overstating it. `verification_status` is a different claim: a claim
-- about this build's relationship with that source. A site that is up and
-- healthy for every reader can still be `blocked-here` for us because this host
-- cannot get past its front door, and conflating the two publishes "the source is
-- down" on a public status endpoint when the truth is "we could not check".
--
-- NULL means the default: verified, or never claimed either way. Deliberately
-- nullable rather than a NOT NULL sentinel — 92 migrations write `sources` rows,
-- and a sentinel would either rewrite all of them or leave every reader handling
-- two spellings of "fine".
ALTER TABLE sources ADD COLUMN verification_status TEXT;  -- 'blocked-here' | NULL
ALTER TABLE sources ADD COLUMN verification_note TEXT;    -- why, in operator words
