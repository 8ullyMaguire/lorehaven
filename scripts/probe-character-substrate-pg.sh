#!/bin/bash
# Apply migration 0103 to a real PostgreSQL 15 and prove the substrate works.
#
# The dialect-parity test compares declared column SETS and index column lists and
# cannot tell whether PostgreSQL *accepts* the DDL. SQLite accepts type errors
# PostgreSQL rejects, so an unreviewed migration passes the suite and fails on the
# second engine. This applies it for real and then exercises §15.3's rule.
#
# The load-bearing test here is the correlation one. §15.3: "Never allow one
# character to satisfy another character's attributes." A work where Alice is a
# protagonist and *Bob* is the vampire must NOT match "protagonist with attribute
# vampire" -- and that is exactly the bug three separate EXISTS clauses would have.
set -u
export PATH=/usr/bin:/bin
export PGPASSWORD=lorehaven PGHOST=127.0.0.1 PGPORT=55433 PGUSER=lorehaven
REPO=/home/alvaro/code-local/rust/lorehaven

psql -d postgres -q -c 'DROP DATABASE IF EXISTS substrate_probe;' 2>&1 | grep -v '^$'
psql -d postgres -q -c 'CREATE DATABASE substrate_probe;' || exit 1
Q() { psql -d substrate_probe -v ON_ERROR_STOP=1 "$@" 2>&1; }

echo "=== 1. the prerequisite tables, as 0003/0011 declare them ==="
Q -q -c "CREATE TABLE works (id UUID PRIMARY KEY);
         CREATE TABLE pseuds (id UUID PRIMARY KEY);
         CREATE TABLE taxonomy_nodes (
             id TEXT PRIMARY KEY,
             kind TEXT NOT NULL,
             canonical TEXT NOT NULL,
             norm TEXT NOT NULL,
             created_at TEXT NOT NULL,
             review_status TEXT NOT NULL DEFAULT 'curated',
             signal_count BIGINT NOT NULL DEFAULT 0);
         CREATE TABLE work_tags (
             work_id UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
             node_id TEXT NOT NULL,
             weight BIGINT NOT NULL DEFAULT 0,
             added_at TEXT NOT NULL,
             PRIMARY KEY (work_id, node_id));" || exit 1
echo "staged"

echo
echo "=== 2. does 0103 apply? ==="
if Q -f "$REPO/migrations/postgres/0103_character_relationships.sql" > /tmp/sub_pg.log 2>&1; then
  echo "APPLIED CLEANLY"
else
  echo "FAILED TO APPLY:"; cat /tmp/sub_pg.log; exit 1
fi

echo
echo "=== 3. declared types -- the parity test is blind to these ==="
Q -c "SELECT table_name, column_name, data_type
      FROM information_schema.columns
      WHERE table_name IN ('work_characters','ship_participants','work_relationships')
      ORDER BY table_name, ordinal_position;"

echo "=== 4. every CHECK landed (SQLite needs triggers for the same rules) ==="
Q -c "SELECT conrelid::regclass AS tbl, count(*) AS checks
      FROM pg_constraint
      WHERE conrelid IN ('work_characters'::regclass,'work_relationships'::regclass,
                         'work_tags'::regclass)
        AND contype = 'c'
      GROUP BY 1 ORDER BY 1;"

# ---------------------------------------------------------------- fixtures
Q -q -c "INSERT INTO works (id) VALUES ('00000000-0000-0000-0000-0000000000a1');"
# Alice: protagonist, vampire. Bob: supporting, BAMF.
Q -q -c "INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at) VALUES
         ('alice','character','Alice','alice','2026-01-01'),
         ('bob','character','Bob','bob','2026-01-01'),
         ('vampire','trait','Vampire','vampire','2026-01-01'),
         ('bamf','trait','BAMF','bamf','2026-01-01'),
         ('ship_ab','ship','Alice/Bob','alice/bob','2026-01-01');"

echo
echo "=== 5. §15.3's rule: Alice=protagonist+vampire, Bob=supporting+BAMF ==="
Q -q -c "INSERT INTO work_characters (work_id, character_node_id, prominence, is_pov, added_at)
         VALUES ('00000000-0000-0000-0000-0000000000a1','alice','protagonist',1,'2026-01-01'),
                ('00000000-0000-0000-0000-0000000000a1','bob','supporting',0,'2026-01-01');"
# The correlation trap: each attribute is attached to its OWN character.
Q -q -c "INSERT INTO work_character_attributes
           (work_id, character_node_id, attribute_node_id, added_at) VALUES
           ('00000000-0000-0000-0000-0000000000a1','alice','vampire','2026-01-01'),
           ('00000000-0000-0000-0000-0000000000a1','bob','bamf','2026-01-01');"

echo "protagonist WITH vampire (must be 1 -- Alice is both):"
Q -c "SELECT count(*) AS must_be_1 FROM work_characters wc
      WHERE wc.work_id='00000000-0000-0000-0000-0000000000a1'
        AND wc.prominence='protagonist'
        AND EXISTS (SELECT 1 FROM work_character_attributes a
                    WHERE a.work_id=wc.work_id AND a.character_node_id=wc.character_node_id
                      AND a.attribute_node_id='vampire');"

echo "protagonist WITH bamf (must be 0 -- only Bob is BAMF, and he is supporting):"
Q -c "SELECT count(*) AS must_be_0 FROM work_characters wc
      WHERE wc.work_id='00000000-0000-0000-0000-0000000000a1'
        AND wc.prominence='protagonist'
        AND EXISTS (SELECT 1 FROM work_character_attributes a
                    WHERE a.work_id=wc.work_id AND a.attribute_node_id='bamf');"
echo "  ^ note that second query deliberately does NOT correlate character_node_id."
echo "    It is the shape three-separate-EXISTS compilation produces, and it"
echo "    returns 1 -- Bob's BAMF satisfies 'protagonist'. That is the bug §15.3 forbids."

echo
echo "=== 6. ship identity is the participant SET ==="
Q -q -c "INSERT INTO ship_participants (ship_node_id, character_node_id) VALUES
         ('ship_ab','alice'), ('ship_ab','bob');"
Q -c "SELECT ship_node_id, count(*) AS participants FROM ship_participants GROUP BY 1;"
echo "(A/B and B/A insert the same two rows, so they cannot diverge.)"

echo
echo "=== 7. relationship type belongs to the WORK, not the ship ==="
Q -q -c "INSERT INTO work_relationships
           (id, work_id, ship_node_id, rel_type, prominence, dynamics, added_at)
         VALUES ('rel1','00000000-0000-0000-0000-0000000000a1','ship_ab','romantic','primary','enemies_to_lovers','2026-01-01');"
Q -c "SELECT wr.rel_type, wr.prominence, wr.dynamics
      FROM work_relationships wr
      JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
      WHERE sp.character_node_id='alice';"

echo "=== 8. journey 12: character X present, NO romantic/sexual relationship with X ==="
Q -c "SELECT count(*) AS matches_journey_12
      FROM work_characters wc
      WHERE wc.work_id='00000000-0000-0000-0000-0000000000a1' AND wc.character_node_id='bob'
        AND NOT EXISTS (
          SELECT 1 FROM work_relationships wr
          JOIN ship_participants sp ON sp.ship_node_id = wr.ship_node_id
          WHERE wr.work_id = wc.work_id
            AND wr.rel_type IN ('romantic','sexual')
            AND sp.character_node_id = wc.character_node_id);"
echo "(Bob IS in a romantic relationship via ship_ab, so this must be 0.)"

echo
echo "=== 9. the CHECKs bite ==="
if Q -q -c "UPDATE work_characters SET prominence='main' WHERE character_node_id='alice';" >/tmp/c9a.log 2>&1; then
  echo "BUG: prominence='main' ACCEPTED"
else echo "rejected bad prominence  <- correct"; fi
if Q -q -c "UPDATE work_relationships SET rel_type='situational' WHERE id='rel1';" >/tmp/c9b.log 2>&1; then
  echo "BUG: bad rel_type ACCEPTED"
else echo "rejected bad rel_type  <- correct"; fi
# An UPDATE against an empty table updates zero rows and "succeeds", so the first
# version of this check passed vacuously. Insert a row first, then assert the
# UPDATE is refused -- and assert the row count too, so a future empty fixture
# cannot make this check pass by doing nothing.
Q -q -c "INSERT INTO work_tags (work_id, node_id, added_at)
         VALUES ('00000000-0000-0000-0000-0000000000a1','vampire','2026-01-01');"
if Q -q -c "UPDATE work_tags SET source='reader' WHERE node_id='vampire';" >/tmp/c9c.log 2>&1; then
  echo "BUG: work_tags.source='reader' ACCEPTED -- ADR 0026 / §49.2 violated"
else echo "rejected work_tags.source='reader'  <- correct (ADR 0026, §49.2)"; fi
echo "work_tags rows still: $(Q -t -A -c "SELECT count(*) FROM work_tags;") (expect 1)"
if Q -q -c "UPDATE work_tags SET source='inferred' WHERE node_id='vampire';" >/tmp/c9d.log 2>&1; then
  echo "BUG: source='inferred' ACCEPTED -- §47.10 / §49.9 forbid ML-inferred tags"
else echo "rejected work_tags.source='inferred'  <- correct (§47.10, §49.9)"; fi

echo
echo "=== 10. an attribute cannot name a character absent from the work ==="
Q -q -c "INSERT INTO works (id) VALUES ('00000000-0000-0000-0000-0000000000b2');"
if Q -q -c "INSERT INTO work_character_attributes
             (work_id, character_node_id, attribute_node_id, added_at)
           VALUES ('00000000-0000-0000-0000-0000000000b2','alice','vampire','2026-01-01');" >/tmp/c10.log 2>&1; then
  echo "BUG: attribute for a character not in the work ACCEPTED"
else echo "rejected orphan attribute  <- correct (composite FK)"; fi

echo
echo "=== 11. cascade: deleting a work removes its characters and relationships ==="
Q -q -c "DELETE FROM works WHERE id='00000000-0000-0000-0000-0000000000a1';"
Q -c "SELECT (SELECT count(*) FROM work_characters WHERE work_id='00000000-0000-0000-0000-0000000000a1') AS chars,
             (SELECT count(*) FROM work_relationships WHERE work_id='00000000-0000-0000-0000-0000000000a1') AS rels,
             (SELECT count(*) FROM work_character_attributes WHERE work_id='00000000-0000-0000-0000-0000000000a1') AS attrs;"
echo "(expect 0, 0, 0 -- attributes cascade via the composite FK to work_characters)"

psql -d postgres -q -c 'DROP DATABASE substrate_probe;' 2>&1 | grep -v '^$'
echo
echo "=== probe database dropped ==="
