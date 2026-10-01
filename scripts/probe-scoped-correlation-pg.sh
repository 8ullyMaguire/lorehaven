#!/bin/bash
# Does §15.4.1.1's `Scoped` actually correlate, and does the correlation matter?
#
# The unit test in crates/domain/tests/query_phase2.rs asserts the SHAPE of the
# compiled SQL (one EXISTS, not two). Shape is necessary but not sufficient, so the
# claim is measured here against a real engine with the exact SQL the renderer emits
# for `relationship:(with:"Alice" with:"Bob" type:romantic)`.
#
# The fixture is built so the two compilations DISAGREE, which is the only way this
# probe means anything:
#
#   work w1: ship duo  {alice,bob}  rel_type = friends
#   work w1: ship poly {alice,bob,carol} rel_type = romantic
#
# Reading "Alice and Bob, romantically":
#   CORRELATED   one relationship row that is romantic AND has both named
#                participants -> the duo is friends, the poly is romantic but has
#                three participants -> 0 relationships match. Answer: 0 works.
#   UNCORRELATED EXISTS(duo has alice and bob) AND EXISTS(any romantic rel)
#                -> both halves are true -> 1 work. WRONG, and wrong invisibly.
#
# A probe whose two variants agreed would pass on an uncorrelated implementation, so
# case 3 asserts the wrong answer is actually what the bad SQL returns -- that is
# what makes case 1 meaningful rather than vacuous.
set -u
export PGPASSWORD=lorehaven PGHOST=127.0.0.1 PGPORT=55433 PGUSER=lorehaven

psql -d postgres -v ON_ERROR_STOP=1 -q <<'SQL' 2>&1
\pset pager off
\echo '=== 15.4.1.1 scoped predicates: does the correlation bind? ==='

DROP TABLE IF EXISTS sp_works, sp_nodes, sp_ships, sp_rels;
CREATE TABLE sp_works (id TEXT PRIMARY KEY);
CREATE TABLE sp_nodes (id TEXT PRIMARY KEY, kind TEXT, norm TEXT);
CREATE TABLE sp_ships (ship_node_id TEXT, character_node_id TEXT);
CREATE TABLE sp_rels (work_id TEXT, ship_node_id TEXT, rel_type TEXT);

INSERT INTO sp_works VALUES ('w1');
INSERT INTO sp_nodes VALUES
    ('alice','character','alice'), ('bob','character','bob'), ('carol','character','carol');
-- The duo is FRIENDS; the poly is ROMANTIC and includes carol.
INSERT INTO sp_ships VALUES ('duo','alice'), ('duo','bob'),
                            ('poly','alice'), ('poly','bob'), ('poly','carol');
INSERT INTO sp_rels VALUES ('w1','duo','friends'), ('w1','poly','romantic');

\echo ''
\echo '--- 1. CORRELATED (what render_scoped emits): expect 0 rows'
-- The three participant clauses and the type clause, all inside ONE EXISTS.
SELECT COUNT(*) AS works_matching
FROM sp_works w
WHERE EXISTS (
    SELECT 1 FROM sp_rels wr WHERE wr.work_id = w.id
      -- (a) every participant of the ship is named
      AND EXISTS (SELECT 1 FROM sp_ships sp WHERE sp.ship_node_id = wr.ship_node_id
                    AND ('/alice//bob//' || '%') LIKE '%/' || sp.character_node_id || '/%')
      -- (b) no participant of the ship is unnamed
      AND NOT EXISTS (SELECT 1 FROM sp_ships sm WHERE sm.ship_node_id = wr.ship_node_id
                        AND ('/alice//bob//' || '%') NOT LIKE '%/' || sm.character_node_id || '/%')
      -- (c) every name given is a participant
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'alice')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'bob')
      -- the row bound, inside the same EXISTS
      AND wr.rel_type = 'romantic'
);
\echo '    ^ MUST be 0. A 1 means the bounds were split into sibling EXISTS.'

\echo ''
\echo '--- 2. the SAME question asked of a relationship that is true: expect 1'
-- The duo IS friends and IS exactly {alice,bob}, so this must match. Case 1 could
-- also return 0 because the predicate never matches anything at all; case 2 is what
-- rules that out.
SELECT COUNT(*) AS works_matching
FROM sp_works w
WHERE EXISTS (
    SELECT 1 FROM sp_rels wr WHERE wr.work_id = w.id
      AND EXISTS (SELECT 1 FROM sp_ships sp WHERE sp.ship_node_id = wr.ship_node_id
                    AND ('/alice//bob//' || '%') LIKE '%/' || sp.character_node_id || '/%')
      AND NOT EXISTS (SELECT 1 FROM sp_ships sm WHERE sm.ship_node_id = wr.ship_node_id
                        AND ('/alice//bob//' || '%') NOT LIKE '%/' || sm.character_node_id || '/%')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'alice')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'bob')
      AND wr.rel_type = 'friends'
);
\echo '    ^ MUST be 1. A 0 means the predicate is dead, and case 1 proved nothing.'

\echo ''
\echo '--- 3. UNCORRELATED (the §15.3 violation): expect 1 -- the WRONG answer'
-- Two sibling EXISTS, which is what the query would compile to without §15.4.1.1.
-- Both halves hold: the duo exists, and a romantic relationship exists. This is the
-- false positive §15.4.1.1 exists to remove, reproduced so case 1 is a real
-- contrast rather than an assertion about SQL nobody can run.
SELECT COUNT(*) AS works_matching
FROM sp_works w
WHERE EXISTS (SELECT 1 FROM sp_rels wr WHERE wr.work_id = w.id
                AND EXISTS (SELECT 1 FROM sp_ships sp WHERE sp.ship_node_id = wr.ship_node_id
                              AND ('/alice//bob//' || '%') LIKE '%/' || sp.character_node_id || '/%')
                AND NOT EXISTS (SELECT 1 FROM sp_ships sm WHERE sm.ship_node_id = wr.ship_node_id
                                  AND ('/alice//bob//' || '%') NOT LIKE '%/' || sm.character_node_id || '/%'))
  AND EXISTS (SELECT 1 FROM sp_rels wr2 WHERE wr2.work_id = w.id
                AND wr2.rel_type = 'romantic');
\echo '    ^ MUST be 1. This is the bug. If it were 0, cases 1 and 2 prove nothing.'

\echo ''
\echo '--- 4. a poly asked for as a duo: expect 0 (clause (b), the M46-01 lesson)'
-- Asking {alice,bob} must not match the poly {alice,bob,carol}: alice and bob are
-- both named, and the poly has a participant that is not. This is the direction
-- that a two-clause implementation gets wrong.
SELECT COUNT(*) AS works_matching
FROM sp_works w
WHERE EXISTS (
    SELECT 1 FROM sp_rels wr WHERE wr.work_id = w.id
      AND EXISTS (SELECT 1 FROM sp_ships sp WHERE sp.ship_node_id = wr.ship_node_id
                    AND ('/alice//bob//' || '%') LIKE '%/' || sp.character_node_id || '/%')
      AND NOT EXISTS (SELECT 1 FROM sp_ships sm WHERE sm.ship_node_id = wr.ship_node_id
                        AND ('/alice//bob//' || '%') NOT LIKE '%/' || sm.character_node_id || '/%')
      AND wr.rel_type = 'romantic'
);
\echo '    ^ MUST be 0. Combined with case 3: case 3 says the poly exists and is romantic,'
\echo '    and this case says naming only two of its three participants does not reach it.'

\echo ''
\echo '--- 5. a poly asked for as the poly: expect 1'
SELECT COUNT(*) AS works_matching
FROM sp_works w
WHERE EXISTS (
    SELECT 1 FROM sp_rels wr WHERE wr.work_id = w.id
      AND EXISTS (SELECT 1 FROM sp_ships sp WHERE sp.ship_node_id = wr.ship_node_id
                    AND ('/alice//bob//carol//' || '%') LIKE '%/' || sp.character_node_id || '/%')
      AND NOT EXISTS (SELECT 1 FROM sp_ships sm WHERE sm.ship_node_id = wr.ship_node_id
                        AND ('/alice//bob//carol//' || '%') NOT LIKE '%/' || sm.character_node_id || '/%')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'alice')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'bob')
      AND EXISTS (SELECT 1 FROM sp_ships sq WHERE sq.ship_node_id = wr.ship_node_id
                    AND sq.character_node_id = 'carol')
      AND wr.rel_type = 'romantic'
);
\echo '    ^ MUST be 1. Clause (c) must not have made a three-part ship unreachable.'

DROP TABLE sp_works, sp_nodes, sp_ships, sp_rels;
SQL
