#!/usr/bin/env bash
# Apply migrations/sqlite/0104_taxonomy_graph.sql to a throwaway SQLite database and
# provoke every constraint the migration claims to add.
#
# Two reasons this exists rather than trusting `migration_catalogue`:
#
#   1. The system `sqlite3` CLI is 3.53.4 and accepts `ALTER TABLE ... ADD
#      CONSTRAINT`. The SQLite the application actually links -- bundled by
#      `libsqlite3-sys 0.30.1` -- is **3.46.0**, which does not. That difference is
#      why the migration uses triggers rather than ADD CONSTRAINT, and it is why
#      this probe must not be the only check: a CLI-only probe passes against a
#      migration the app cannot apply. `crates/app/tests/migrate_through_0104.rs`
#      runs the real chain through sqlx and is the gate that caught it.
#   2. Foreign keys need `PRAGMA foreign_keys=ON`, which is OFF by default in a bare
#      `sqlite3` session. `crates/db/src/lib.rs` sets `.foreign_keys(true)` on its
#      pool, so the application does enforce them; this probe has to say so
#      explicitly or it reports a false "NOT ENFORCED" for constraints the app holds.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$(mktemp -d)"
DB="$DIR/probe.sqlite"
trap 'rm -rf "$DIR"' EXIT

# `sqlite3 DB "SQL"` executes the argument and IGNORES stdin. So the pragma cannot
# be passed as an argument alongside heredoc input -- doing so silently discards
# every statement on stdin, which looks like "no such table" rather than a usage
# error. Hence: prepend the pragma to whatever SQL is piped in, and never pass SQL
# as an argument at the same time as stdin.
run() { command sqlite3 "$DB" "PRAGMA foreign_keys=ON; $(cat)" ; }

# Same rule for the migration file: fold the pragma into the stream.
run_file() { { echo "PRAGMA foreign_keys=ON;"; cat "$1"; } | command sqlite3 "$DB"; }

# A stand-in for what 0011 and 0082 create, with the same column types the real
# migration runs against -- notably `id TEXT`, which is why the plan's
# `merged_into INTEGER` is wrong.
run <<'SQL'
CREATE TABLE taxonomy_nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    canonical TEXT NOT NULL,
    norm TEXT NOT NULL,
    created_at TEXT NOT NULL,
    review_status TEXT NOT NULL DEFAULT 'curated',
    signal_count INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX taxonomy_nodes_kind_norm ON taxonomy_nodes (kind, norm);
INSERT INTO taxonomy_nodes (id, kind, canonical, norm, created_at)
VALUES ('n1', 'character', 'Alice', 'alice', '2026-01-01T00:00:00Z'),
       ('n2', 'character', 'Spike', 'spike', '2026-01-01T00:00:00Z');
SQL

echo "--- sqlite CLI $(sqlite3 --version | cut -d' ' -f1)"
echo "    (the app links 3.46.0 via libsqlite3-sys; see migrate_through_0104.rs)"
echo "--- applying 0104"
run_file "$REPO/migrations/sqlite/0104_taxonomy_graph.sql"
echo "APPLIED OK"

fail=0
sql() { command sqlite3 "$DB" "PRAGMA foreign_keys=ON; $1"; }

expect_refused() {
  if sql "$2" >/dev/null 2>&1; then
    echo "  NOT ENFORCED   $1"
    fail=1
  else
    echo "  refused        $1"
  fi
}
expect_accepted() {
  if sql "$2" >/dev/null 2>&1; then
    echo "  accepted       $1"
  else
    echo "  WRONGLY REFUSED  $1"
    fail=1
  fi
}

echo "--- an existing row defaults to active, not NULL"
echo "  n1 status=$(sql "SELECT status FROM taxonomy_nodes WHERE id='n1';" | tr -d ' ')"
expect_accepted "an existing row can move to pending" \
  "UPDATE taxonomy_nodes SET status='pending' WHERE id='n1';"

echo "--- status CHECK"
expect_refused "status='bogus'" \
  "UPDATE taxonomy_nodes SET status='bogus' WHERE id='n1';"

echo "--- merged_into is inseparable from status='merged'"
expect_refused "merged with merged_into NULL" \
  "UPDATE taxonomy_nodes SET status='merged' WHERE id='n1';"
expect_refused "active with merged_into set" \
  "UPDATE taxonomy_nodes SET status='active', merged_into='n2' WHERE id='n1';"
expect_accepted "merged with merged_into set" \
  "UPDATE taxonomy_nodes SET status='merged', merged_into='n2' WHERE id='n1';"
expect_refused "merged_into pointing at a node that does not exist" \
  "UPDATE taxonomy_nodes SET status='merged', merged_into='nope' WHERE id='n1';"

echo "--- scoping"
expect_refused "a node scoped to itself" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','n1','2026-01-01T00:00:00Z');"
expect_refused "scoping to a node that does not exist" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','nope','2026-01-01T00:00:00Z');"
expect_accepted "a node scoped to another real node" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','n2','2026-01-01T00:00:00Z');"

echo "--- edges"
expect_refused "a self edge" \
  "INSERT INTO taxonomy_edges VALUES ('n1','n1','parent',NULL,'2026-01-01T00:00:00Z');"
expect_refused "an unknown relation" \
  "INSERT INTO taxonomy_edges VALUES ('n1','n2','hates',NULL,'2026-01-01T00:00:00Z');"
expect_refused "an edge to a node that does not exist" \
  "INSERT INTO taxonomy_edges VALUES ('n1','nope','parent',NULL,'2026-01-01T00:00:00Z');"
expect_accepted "a known relation between real nodes" \
  "INSERT INTO taxonomy_edges VALUES ('n1','n2','parent',NULL,'2026-01-01T00:00:00Z');"

echo "--- closure"
expect_refused "a negative depth" \
  "INSERT INTO taxonomy_closure VALUES ('n1','n2','parent',-1,'2026-01-01T00:00:00Z');"
expect_accepted "a depth-0 self row, which is what +children needs" \
  "INSERT INTO taxonomy_closure VALUES ('n1','n1','parent',0,'2026-01-01T00:00:00Z');"
expect_accepted "the join expansion actually issues" \
  "SELECT descendant_id FROM taxonomy_closure WHERE ancestor_id='n1' AND rel='parent';"

echo
if [ "$fail" -eq 0 ]; then
  echo "all constraints enforced"
else
  echo "AT LEAST ONE CONSTRAINT IS NOT ENFORCED"
  exit 1
fi
