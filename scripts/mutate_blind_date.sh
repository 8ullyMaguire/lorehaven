#!/usr/bin/env bash
# Mutation gate for gap B (blind_date).
#
# Each mutation removes exactly one eligibility clause, and the corresponding test MUST
# go red. A GREEN(BAD) line means the clause is not covered -- see the note in
# crates/app/tests/blind_date.rs about the fixture that passed for the wrong reason.
#
# The H-numbers here are local to this file; gap F's harness uses its own.
set -uo pipefail
cd ~/code-local/rust/lorehaven
export PATH="$HOME/.cargo/bin:$PATH"
SRC=crates/db/src/discovery.rs

# Apply, test, restore. `run_mutation` prints RED or GREEN(BAD).
#
# Two harness bugs cost real time on gap F's first run and are guarded against here:
#   * a compiler error spans many lines, so the whole output is grepped rather than a
#     single `head -1` line -- otherwise a compile failure reads as GREEN;
#   * a compile failure caused by a *deliberately unbalanced* mutation is not evidence
#     that the clause is covered, because the test never ran. Those mutations are marked
#     and their result is reported separately.
run_mutation() {
  local name="$1" old="$2" new="$3" expect_run="${4:-yes}"
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
  out=$(cargo test -p lorehaven-app --test blind_date --no-fail-fast -- \
        --test-threads=4 2>&1)
  python3 - "$SRC" "$new" "$old" <<'PY'
import sys
path, new, old = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
open(path, "w").write(s.replace(new, old, 1))
PY
  if grep -qE '^error\[|^error: could not compile' <<<"$out"; then
    if [ "$expect_run" = "no" ]; then
      echo "$name -> RED (compile error, expected)"
    else
      echo "$name -> RED (compile error -- the tests never ran)"
    fi
    return
  fi
  if grep -qE '^test result: FAILED|\.\.\. FAILED' <<<"$out"; then
    echo "$name -> RED"
  else
    echo "$name -> GREEN(BAD)"
  fi
}

# B1 the seed stops depending on the day, so the pick never changes.
run_mutation "B1 date removed from seed" \
  "blind_date_seed(account, today)" "blind_date_seed(account, \"\")"

# B2 the seed stops depending on the account, so every reader gets the same work.
run_mutation "B2 account removed from seed" \
  "blind_date_seed(account, today)" "blind_date_seed(\"\", today)"

# B3 a bookmarked work becomes eligible again.
#
# `AND 1 = 0` on the subquery predicate, NOT `AND (X OR 1 = 0)`. Both were tried and the
# second is a no-op: `X OR false` is `X`, so the exclusion stays active and the mutation
# reads GREEN(BAD) while the clause is fully intact. `1 = 0` on the predicate makes the
# subquery return the empty set, which is the only form that actually disables it -- and
# it keeps `{account}` used, so the build succeeds and the test really runs.
run_mutation "B3 bookmark exclusion dropped" \
  "WHERE account_id = '{account}' AND subject_type = 'work'" \
  "WHERE account_id = '{account}' AND subject_type = 'work' AND 1 = 0"

# B4 the author exclusion goes, so an author the reader has finished keeps appearing.
# Same `AND 1 = 0` form: empty subquery, so the NOT IN admits every owner.
run_mutation "B4 author exclusion dropped" \
  "AND rs.status = 'finished'" "AND rs.status = 'finished' AND 1 = 0"

# B5 unlisted works become listable.
run_mutation "B5 visibility filter dropped" \
  "AND w.visibility = 'public'" "AND w.visibility IS NOT NULL"

# B6 a draft becomes eligible.
run_mutation "B6 lifecycle filter dropped" \
  "WHERE w.lifecycle = 'published'
          AND w.visibility" "WHERE w.visibility"

# B7 a future work becomes eligible today.
run_mutation "B7 published_at gate dropped" \
  "AND date(COALESCE(w.published_at, w.created_at)) <= date('{today}')" \
  "AND date(COALESCE(w.published_at, w.created_at)) IS NOT NULL"

# B8 a deleted work stays in the pool.
run_mutation "B8 deleted_at filter dropped" \
  "AND w.deleted_at IS NULL" "AND (w.deleted_at IS NULL OR w.deleted_at IS NOT NULL)"

# B9 the ordering stops depending on the seed's *contents*.
#
# Two earlier attempts were invalid, both because warnings are denied in this repo and
# the build failed before any test ran: `min_by_key(|id| id.clone())` leaves
# `blind_date_order_key` uncalled (dead_code), and `.chain(b"")` leaves `seed` unused.
# `.chain([seed.len()])` keeps both variables live while making the hash independent of
# the account and date -- so the order stops varying per reader and per day, which is
# exactly what B1/B2 and `the_pick_is_the_one_the_seed_dictates` assert against.
run_mutation "B9 seed dropped from order key" \
  ".chain(seed.as_bytes())" \
  ".chain([seed.len()])"

# B10 the ordering becomes reverse-sorted, which still returns a work but not the one
# the seed dictates.
run_mutation "B10 ordering reversed" \
  "min_by_key(|id| blind_date_order_key(id, &seed))" \
  "max_by_key(|id| blind_date_order_key(id, &seed))"

echo "MUTATIONS_DONE"
echo "final: $(cargo test -p lorehaven-app --test blind_date -- --test-threads=4 2>&1 | grep -E '^test result' | head -1)"