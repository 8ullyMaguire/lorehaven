#!/usr/bin/env bash
# Mutation gate for gap F (hidden_classics).
#
# Each mutation breaks exactly one rule, and the corresponding test MUST go red. A
# GREEN(BAD) line means the rule is not actually covered and the fixture needs work --
# see the note in crates/app/tests/hidden_classics.rs about why three of these
# survived the first pass.
set -uo pipefail
cd ~/code-local/rust/lorehaven
export PATH="$HOME/.cargo/bin:$PATH"
SRC=crates/db/src/rec_strategy.rs

run_mutation() {
  local name="$1" old="$2" new="$3"
  if ! python3 - "$SRC" "$old" "$new" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
if old not in s:
    sys.exit(3)
open(path, "w").write(s.replace(old, new, 1))
PY
  then
    echo "$name -> SKIP (pattern not found)"
    return
  fi
  local out
  out=$(cargo test -p lorehaven-app --test hidden_classics --no-fail-fast -- \
        --test-threads=4 2>&1)
  # `grep -q` over the whole output, not a single captured line: a compiler error spans
  # many lines and the diagnostic line is not always the first match.
  if grep -qE '^error\[|^error: could not compile' <<<"$out"; then
    echo "$name -> RED (compile error)"
    python3 - "$SRC" "$new" "$old" <<'PY'
import sys
path, new, old = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
open(path, "w").write(s.replace(new, old, 1))
PY
    return
  fi
  out=$(grep -E '^test result' <<<"$out" | head -1)
  python3 - "$SRC" "$new" "$old" <<'PY'
import sys
path, new, old = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
open(path, "w").write(s.replace(new, old, 1))
PY
  case "$out" in
    *FAILED*)  echo "$name -> RED" ;;
    *)         echo "$name -> GREEN(BAD)" ;;
  esac
}

run_mutation "H1 DISTINCT readers dropped" \
  'COUNT(DISTINCT v.viewer_hash)' 'COUNT(v.viewer_hash)'
run_mutation "H2 automated views counted" \
  'WHERE {view_join} AND v.is_automated = 0' 'WHERE {view_join} AND TRUE'
run_mutation "H3 reader floor removed" \
  'WHERE readers >= {min_readers}' 'WHERE readers >= 1'
run_mutation "H4 zero completions allowed" \
  'AND completions > 0' 'AND completions >= 0'
run_mutation "H5 log replaced by linear" \
  'rate / (1.0 + (1.0 + readers as f64).log10())' 'rate / readers as f64'

# H6 and H7 must be SCOPED to the `engagement` CTE. An unscoped
# `AND w.lifecycle = 'published'` matches the first sibling strategy in the file, which
# these tests never touch, so the mutation reads GREEN(BAD) while the clause under test
# is completely intact -- a false survivor, which is worse than a flaky one.
run_mutation "H6 lifecycle filter dropped" \
  "WHERE w.lifecycle = 'published'
                      AND w.id NOT IN (" \
  "WHERE w.id NOT IN ("

# `AND 1 = 0` on the subquery predicate, NOT `AND (X OR 1 = 0)`. `X OR false` is `X`, so
# that form leaves the exclusion fully active and reports a false survivor. `1 = 0` on
# the predicate returns the empty set -- the exclusion is genuinely gone -- while keeping
# `{account}` used, so the build still succeeds and the test really runs.
run_mutation "H7 bookmark exclusion dropped" \
  "SELECT subject_id FROM bookmarks
                          WHERE account_id = '{account}' AND subject_type = 'work'" \
  "SELECT subject_id FROM bookmarks
                          WHERE account_id = '{account}' AND subject_type = 'work' AND 1 = 0"

# H8: ties break on work id. Needs two works with identical (readers, completions).
run_mutation "H8 tie-break reversed" \
  '.then_with(|| a.0.cmp(&b.0))' '.then_with(|| b.0.cmp(&a.0))'

# H9: the completion subquery stops filtering on subject_type, so completions of any
# kind count toward the quality rate.
run_mutation "H9 subject_type filter dropped" \
  "WHERE rs.subject_type = 'work'
                              AND rs.subject_id = w.id" \
  "WHERE rs.subject_id = w.id"

# H10: the tie-break disappears entirely, leaving the order to the database.
run_mutation "H10 tie-break removed" \
  ".then_with(|| a.0.cmp(&b.0))" ""

echo "MUTATIONS_DONE"
echo "final: $(cargo test -p lorehaven-app --test hidden_classics -- --test-threads=4 2>&1 | grep -E '^test result' | head -1)"
