#!/bin/bash
# Does `relationship:` match the right pairings?
#
# `QueryField::Relationship` compares a ship's participant SET against the
# reader's value in BOTH directions, using `instr` on a slash-delimited string.
# That is easy to reason about and easy to get wrong, so it is measured here on
# real PostgreSQL 15 against the shape 0103 creates.
#
# The cases that matter:
#   duo asked, duo stored   -> match
#   duo asked, poly stored  -> NO match  (the NOT EXISTS half)
#   poly asked, duo stored  -> NO match  (the EXISTS half)
#
# A version that tested only one direction returns the wrong answer on the third.
set -u
export PGPASSWORD=lorehaven PGHOST=127.0.0.1 PGPORT=55433 PGUSER=lorehaven

psql -d postgres -v ON_ERROR_STOP=1 -q <<'SQL' 2>&1
\pset pager off
\echo '=== relationship: does the two-direction set comparison hold? ==='

DROP TABLE IF EXISTS wr_probe_works, wr_probe_nodes, wr_probe_ships, wr_probe_rels;
CREATE TABLE wr_probe_works (id TEXT PRIMARY KEY);
CREATE TABLE wr_probe_nodes (id TEXT PRIMARY KEY, kind TEXT, norm TEXT);
CREATE TABLE wr_probe_ships (ship_node_id TEXT, character_node_id TEXT);
CREATE TABLE wr_probe_rels (work_id TEXT, ship_node_id TEXT, rel_type TEXT);

INSERT INTO wr_probe_works VALUES ('w1');
INSERT INTO wr_probe_nodes VALUES
    ('alice','character','alice'), ('bob','character','bob'), ('carol','character','carol');
-- a duo ship {alice,bob} and a poly ship {alice,bob,carol}
INSERT INTO wr_probe_ships VALUES ('duo','alice'), ('duo','bob');
INSERT INTO wr_probe_ships VALUES ('poly','alice'), ('poly','bob'), ('poly','carol');
INSERT INTO wr_probe_rels VALUES ('w1','duo','romantic'), ('w1','poly','platonic');

\echo ''
\echo '--- 1. duo asked, duo and poly stored: expect exactly the duo'
SELECT DISTINCT wr.ship_node_id
FROM wr_probe_rels wr
WHERE wr.work_id = 'w1'
  AND EXISTS (SELECT 1 FROM wr_probe_ships sp
              WHERE sp.ship_node_id = wr.ship_node_id
                AND '%/alice//bob/%' LIKE '%/' || sp.character_node_id || '/%')
  AND NOT EXISTS (SELECT 1 FROM wr_probe_ships sm
                  WHERE sm.ship_node_id = wr.ship_node_id
                    AND '%/alice//bob/%' NOT LIKE '%/' || sm.character_node_id || '/%');
\echo '    ^ MUST be exactly one row, "duo". Two rows means the NOT EXISTS half is missing.'

\echo ''
\echo '--- 2. reversed order B/A: expect the same duo, since the set is the identity'
SELECT DISTINCT wr.ship_node_id
FROM wr_probe_rels wr
WHERE wr.work_id = 'w1'
  AND EXISTS (SELECT 1 FROM wr_probe_ships sp
              WHERE sp.ship_node_id = wr.ship_node_id
                AND '%/bob//alice/%' LIKE '%/' || sp.character_node_id || '/%')
  AND NOT EXISTS (SELECT 1 FROM wr_probe_ships sm
                  WHERE sm.ship_node_id = wr.ship_node_id
                    AND '%/bob//alice/%' NOT LIKE '%/' || sm.character_node_id || '/%');

\echo ''
\echo '--- 3. poly asked: expect exactly the poly ship'
SELECT DISTINCT wr.ship_node_id
FROM wr_probe_rels wr
WHERE wr.work_id = 'w1'
  AND EXISTS (SELECT 1 FROM wr_probe_ships sp
              WHERE sp.ship_node_id = wr.ship_node_id
                AND '%/alice//bob//carol/%' LIKE '%/' || sp.character_node_id || '/%')
  AND NOT EXISTS (SELECT 1 FROM wr_probe_ships sm
                  WHERE sm.ship_node_id = wr.ship_node_id
                    AND '%/alice//bob//carol/%' NOT LIKE '%/' || sm.character_node_id || '/%');

\echo ''
\echo '--- 4. the delimiter is load-bearing: alice must not match alice_v2'
SELECT ('%/alice_v2//bob/%' LIKE '%/alice/%')   AS prefix_should_be_false,
       ('%/alice_v2//bob/%' LIKE '%/alice_v2/%') AS exact_should_be_true;
\echo '    ^ without the slashes, alice_v2 would satisfy a search for alice.'

DROP TABLE wr_probe_works, wr_probe_nodes, wr_probe_ships, wr_probe_rels;
\echo ''
\echo '=== probe tables dropped ==='
SQL