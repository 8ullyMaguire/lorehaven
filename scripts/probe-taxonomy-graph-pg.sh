#!/usr/bin/env bash
# Apply migrations/postgres/0104_taxonomy_graph.sql to a throwaway PostgreSQL
# database and provoke every constraint it claims to add.
#
# The SQLite twin is `probe-taxonomy-graph-sqlite.sh`. This one exists because the
# two dialects differ in a way that decides whether the migration is even valid:
#
#   * `ALTER TABLE ... ADD CONSTRAINT` is native PostgreSQL and, unlike SQLite, is
#     enforced against EXISTING rows. A CHECK that only guards new writes passes
#     here and would have been the wrong gate on the other engine.
#   * `merged_into` must be TEXT. The plan's `merged_into INTEGER` cannot reference
#     `taxonomy_nodes.id` (TEXT), and PostgreSQL has no implicit integer->text
#     coercion inside a foreign key, so the plan's DDL is rejected outright here.
#     That is checked below rather than assumed.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${LOREHAVEN_TEST_PG_URL:?set LOREHAVEN_TEST_PG_URL to the instance under test}"

DB="lh_probe_taxgraph_$$"

# `psql -d <url>` per invocation, so host, port, user and password are always read
# by psql itself and never re-parsed here. Re-parsing is where this script first
# broke: `${PGURL##*@}` is greedy, so it swallowed the host, port and database along
# with the password and produced PGPORT="55433/postgres" plus a garbage PGPASSWORD
# -- which surfaced only as psql refusing to connect, with nothing pointing at the
# shell arithmetic that caused it.
#
# The admin URL is the test URL with its database swapped for `postgres`; the probe
# URL swaps in the throwaway name. Matching on the last `/` avoids having to match
# the password, which may contain regex metacharacters.
ADMIN="$(printf '%s' "$LOREHAVEN_TEST_PG_URL" | sed -E 's#^(.*)/[^/]*$#\1/postgres#')"
PROBE="$(printf '%s' "$ADMIN" | sed -E "s#^(.*)/[^/]*\$#\1/$DB#")"

# -t -A so a value read back is the bare value, not a header plus a row count. Error
# text still comes through, which is what the DDL check below greps for.
q() { psql -X -q -t -A -v ON_ERROR_STOP=1 -d "$PROBE" -c "$1" 2>&1; }
adm() { psql -X -q -t -A -v ON_ERROR_STOP=1 -d "$ADMIN" -c "$1" 2>&1; }

cleanup() { adm "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1 || true; }
trap cleanup EXIT

cleanup
adm "CREATE DATABASE $DB" >/dev/null
echo "--- postgres $(adm 'SHOW server_version')"

# A stand-in for what 0011 and 0082 create, with the real column types -- `id TEXT`
# is the whole reason the plan's INTEGER reference is wrong.
q "
CREATE TABLE taxonomy_nodes (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    canonical     TEXT NOT NULL,
    norm          TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    review_status TEXT NOT NULL DEFAULT 'curated',
    signal_count  INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX taxonomy_nodes_kind_norm ON taxonomy_nodes (kind, norm);
INSERT INTO taxonomy_nodes (id, kind, canonical, norm) VALUES
    ('n1', 'character', 'Alice', 'alice'),
    ('n2', 'character', 'Spike', 'spike');
" >/dev/null

# The plan writes `merged_into INTEGER REFERENCES taxonomy_nodes(id)`, and
# `taxonomy_nodes.id` is TEXT. PostgreSQL ACCEPTS that DDL -- measured, not assumed;
# the probe's first version asserted it would be rejected and was wrong. Accepting
# the DDL is not the same as the column working, so what matters is whether the
# constraint is enforced and whether it can ever match a real node id.
echo "--- the plan's INTEGER foreign key"
plan_err="$(q "ALTER TABLE taxonomy_nodes ADD COLUMN merged_into_bad INTEGER REFERENCES taxonomy_nodes(id);" || true)"
if printf '%s' "$plan_err" | grep -qi 'incompatible types'; then
  echo "  DDL refused    the plan's INTEGER foreign key to a TEXT id:"
  printf '%s\n' "$plan_err" | grep -i 'detail\|error' | sed 's/^/                 /'
else
  echo "  DDL accepted   merged_into INTEGER -> taxonomy_nodes.id (TEXT)"
  echo "  => the migration's TEXT correction would be unnecessary; re-check why"
  plan_err=""
fi

if [ -n "$plan_err" ]; then
  # The column may or may not exist depending on how far PostgreSQL got before
  # refusing, so the follow-up is guarded rather than assumed.
  if q "SELECT merged_into_bad FROM taxonomy_nodes LIMIT 1" >/dev/null 2>&1; then
    q "ALTER TABLE taxonomy_nodes DROP COLUMN merged_into_bad;" >/dev/null 2>&1
    echo "  cleaned up     the partially-added column"
  fi
fi

echo "--- applying 0104"
if psql -X -q -v ON_ERROR_STOP=1 -d "$PROBE" -f "$REPO/migrations/postgres/0104_taxonomy_graph.sql" >/dev/null 2>&1; then
  echo "  APPLIED OK"
else
  echo "  APPLY FAILED"
  psql -X -q -v ON_ERROR_STOP=1 -d "$PROBE" -f "$REPO/migrations/postgres/0104_taxonomy_graph.sql" 2>&1 | head -5
  exit 1
fi

fail=0
expect_refused() {
  if q "$2" >/dev/null 2>&1; then
    echo "  NOT ENFORCED   $1"; fail=1
  else
    echo "  refused        $1"
  fi
}
expect_accepted() {
  if q "$2" >/dev/null 2>&1; then
    echo "  accepted       $1"
  else
    echo "  WRONGLY REFUSED  $1"; fail=1
  fi
}

echo "--- an existing row defaults to active"
echo "  n1 status=$(q "SELECT status FROM taxonomy_nodes WHERE id='n1'" | tr -d ' ')"
expect_accepted "an existing row can move to pending" \
  "UPDATE taxonomy_nodes SET status='pending' WHERE id='n1';"

echo "--- status CHECK"
expect_refused "status='bogus'" "UPDATE taxonomy_nodes SET status='bogus' WHERE id='n1';"

echo "--- merged_into is inseparable from status='merged'"
expect_refused "merged with merged_into NULL" "UPDATE taxonomy_nodes SET status='merged' WHERE id='n1';"
expect_refused "active with merged_into set" "UPDATE taxonomy_nodes SET status='active', merged_into='n2' WHERE id='n1';"
expect_accepted "merged with merged_into set" "UPDATE taxonomy_nodes SET status='merged', merged_into='n2' WHERE id='n1';"
expect_refused "merged_into pointing at a node that does not exist" \
  "UPDATE taxonomy_nodes SET status='merged', merged_into='nope' WHERE id='n1';"

echo "--- the ON DELETE SET NULL interaction the merge CHECK exists for"
# n1 is merged into n2. Deleting n2 sets merged_into to NULL while status stays
# 'merged', which the CHECK must refuse. Were that delete allowed, a name a curator
# retired would silently come back to life.
expect_refused "deleting a merge target that a node still points at" \
  "DELETE FROM taxonomy_nodes WHERE id='n2';"
q "UPDATE taxonomy_nodes SET status='active', merged_into=NULL WHERE id='n1';" >/dev/null
expect_accepted "with no merge outstanding, the target is deletable" \
  "DELETE FROM taxonomy_nodes WHERE id='n2';"

echo "--- scoping"
q "INSERT INTO taxonomy_nodes (id, kind, canonical, norm) VALUES ('n3','fandom','Buffy','buffy');" >/dev/null
q "INSERT INTO taxonomy_nodes (id, kind, canonical, norm) VALUES ('n4','fandom','Angel','angel');" >/dev/null
expect_refused "a node scoped to itself" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','n1', now());"
expect_refused "scoping to a node that does not exist" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','nope', now());"
expect_accepted "a character scoped to one fandom" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','n3', now());"
expect_accepted "the same character scoped to a second fandom (crossover)" \
  "INSERT INTO taxonomy_node_scope VALUES ('n1','n4', now());"

echo "--- edges"
expect_refused "a self edge" "INSERT INTO taxonomy_edges VALUES ('n1','n1','parent',NULL,now());"
expect_refused "an unknown relation" "INSERT INTO taxonomy_edges VALUES ('n1','n3','hates',NULL,now());"
expect_refused "an edge to a node that does not exist" \
  "INSERT INTO taxonomy_edges VALUES ('n1','nope','parent',NULL,now());"
expect_accepted "a known relation between real nodes" \
  "INSERT INTO taxonomy_edges VALUES ('n1','n3','parent',NULL,now());"

echo "--- closure"
expect_refused "a negative depth" "INSERT INTO taxonomy_closure VALUES ('n1','n3','parent',-1,now());"
expect_accepted "a depth-0 self row, which is what +children needs" \
  "INSERT INTO taxonomy_closure VALUES ('n1','n1','parent',0,now());"
expect_accepted "the join expansion actually issues" \
  "SELECT descendant_id FROM taxonomy_closure WHERE ancestor_id='n1' AND rel='parent';"

echo
if [ "$fail" -eq 0 ]; then
  echo "all constraints enforced"
else
  echo "AT LEAST ONE CONSTRAINT IS NOT ENFORCED"
  exit 1
fi
