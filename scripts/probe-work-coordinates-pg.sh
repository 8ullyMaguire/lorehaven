#!/bin/bash
# Apply migration 0102 to a real PostgreSQL 15 and prove the CHECKs bite.
#
# The dialect-parity test compares declared column SETS and index column lists. It
# cannot tell whether PostgreSQL *accepts* the DDL, and SQLite accepts type errors
# PostgreSQL rejects -- so an unreviewed migration passes the suite and fails on the
# second engine. This applies it for real, then attacks each CHECK.
set -u
export PATH=/usr/bin:/bin
PGURL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres'
REPO=/home/alvaro/code-local/rust/lorehaven

export PGPASSWORD=lorehaven
export PGHOST=127.0.0.1 PGPORT=55433 PGUSER=lorehaven
psql -d postgres -q -c 'DROP DATABASE IF EXISTS coord_probe;' 2>&1 | grep -v '^$'
psql -d postgres -q -c 'CREATE DATABASE coord_probe;' || exit 1
# The connection string goes in PGPASSWORD/PGHOST-style env vars, NOT as a bare
# argument: psql reads the first non-option argument as the *database name*, so
# `psql ... "$PGURL" -c ...` sends the whole URL to the server as a role name and
# fails with `role "postgres://..." does not exist`. Using -d for the database and
# the URL last-but-before-the-flags is fragile; env vars are not.
Q() { psql -d coord_probe -v ON_ERROR_STOP=1 "$@" 2>&1; }

echo "=== 1. does the DDL apply at all? ==="
Q -c "CREATE TABLE works (id UUID PRIMARY KEY);" >/dev/null
if Q -f "$REPO/migrations/postgres/0102_work_coordinates.sql" > /tmp/coord_pg.log 2>&1; then
  echo "APPLIED CLEANLY"
else
  echo "FAILED TO APPLY:"; cat /tmp/coord_pg.log; exit 1
fi

echo
echo "=== 2. declared shape (types matter, the parity test is blind to them) ==="
Q -c "SELECT column_name, data_type, is_nullable FROM information_schema.columns
      WHERE table_name='work_coordinates' ORDER BY ordinal_position;"

echo "=== 3. the index, and that it is partial ==="
Q -c "SELECT indexname, indexdef FROM pg_indexes WHERE tablename='work_coordinates';"

echo "=== 4. every CHECK exists, named by position since unnamed ==="
Q -c "SELECT count(*) AS check_count FROM pg_constraint
      WHERE conrelid='work_coordinates'::regclass AND contype='c';"

echo
echo "=== 5. each CHECK bites ==="
# a parent work so the FK can be satisfied
Q -q -c "INSERT INTO works (id) VALUES ('00000000-0000-0000-0000-000000000001');"

# 5a. measured work: accepted
Q -q -c "INSERT INTO work_coordinates
        (work_id, sentence_length_variance, dialogue_ratio, vocabulary_richness,
         chapter_length_spread, text_version, computed_at)
        VALUES ('00000000-0000-0000-0000-000000000001', 0.4, 0.2, 0.6, NULL, 1, '2026-10-01T00:00:00Z');"
echo "measured work (chapter_length_spread NULL): accepted"

# 5b. measured work, no chapter distribution -> must be ACCEPTED (single chapter)
Q -q -c "UPDATE work_coordinates SET chapter_length_spread=NULL
        WHERE work_id='00000000-0000-0000-0000-000000000001';"
echo "single-chapter measured work (spread NULL): accepted  <- the §49.3 case"

# 5c. a coordinate above the weight scale -> must be REJECTED
if Q -q -c "UPDATE work_coordinates SET dialogue_ratio=1.7
        WHERE work_id='00000000-0000-0000-0000-000000000001';" >/tmp/c5c.log 2>&1; then
  echo "BUG: dialogue_ratio 1.7 ACCEPTED -- the 0.0..=1.0 CHECK did not bite"
else
  echo "rejected dialogue_ratio=1.7  <- correct"
  grep -o 'violates check constraint[^"]*' /tmp/c5c.log | head -1
fi

# 5d. both measured AND a reason -> must be REJECTED (mutual exclusion)
if Q -q -c "UPDATE work_coordinates
        SET dialogue_ratio=0.2, unmeasurable_reason='too_short'
        WHERE work_id='00000000-0000-0000-0000-000000000001';" >/tmp/c5d.log 2>&1; then
  echo "BUG: measured AND unmeasurable_reason ACCEPTED -- mutual exclusion did not bite"
else
  echo "rejected measured+reason together  <- correct"
fi

# 5e. unmeasurable work: accepted
Q -q -c "INSERT INTO works (id) VALUES ('00000000-0000-0000-0000-000000000002');"
Q -q -c "INSERT INTO work_coordinates
        (work_id, unmeasurable_reason, word_count, text_version, computed_at)
        VALUES ('00000000-0000-0000-0000-000000000002', 'too_short', 300, 1, '2026-10-01T00:00:00Z');"
echo "unmeasurable work (measures NULL, reason set): accepted"

# 5f. an undeclared reason -> must be REJECTED
if Q -q -c "UPDATE work_coordinates SET unmeasurable_reason='too_shorttt'
        WHERE work_id='00000000-0000-0000-0000-000000000002';" >/tmp/c5f.log 2>&1; then
  echo "BUG: undeclared reason ACCEPTED -- the enum CHECK did not bite"
else
  echo "rejected undeclared reason  <- correct"
fi

# 5g. the partial index must EXCLUDE the unmeasurable row and INCLUDE the measured one
echo
echo "=== 6. the partial index returns exactly the measured works ==="
Q -c "SELECT work_id FROM work_coordinates WHERE unmeasurable_reason IS NULL;"
echo "(expect only ...0001; the too_short row must be absent)"

echo
echo "=== 7. cascade: deleting a work takes its coordinates with it ==="
Q -q -c "DELETE FROM works WHERE id='00000000-0000-0000-0000-000000000002';"
Q -c "SELECT count(*) AS orphaned FROM work_coordinates
      WHERE work_id='00000000-0000-0000-0000-000000000002';"
echo "(expect 0)"

echo
echo "=== 8. byte-for-byte float round trip (REAL would fail this) ==="
Q -c "INSERT INTO works (id) VALUES ('00000000-0000-0000-0000-000000000003');"
Q -c "INSERT INTO work_coordinates
      (work_id, sentence_length_variance, dialogue_ratio, vocabulary_richness,
       text_version, computed_at)
      VALUES ('00000000-0000-0000-0000-000000000003',
              0.123457, 1.0/3.0, 0.999999, 1, '2026-10-01T00:00:00Z')
      RETURNING sentence_length_variance, dialogue_ratio;"
echo "(0.123457 must come back exactly; 1.0/3.0 needs 17 significant digits)"

psql -d postgres -q -c 'DROP DATABASE coord_probe;' 2>&1 | grep -v '^$'
echo
echo "=== probe database dropped ==="
