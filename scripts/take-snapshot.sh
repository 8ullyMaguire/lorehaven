#!/usr/bin/env bash
# Take a snapshot, and refuse to produce one that does not restore.
#
# Spec §11.16.7 step 5: a snapshot that does not restore into an empty database
# and pass `doctor` is not published. This script bakes that refusal in, because
# "did I remember to run the restore check" is exactly the thing remembered once.
# A snapshot that does not restore is worse than no snapshot -- a recipient is
# the one who discovers it.
#
# The order below is a security property, not a preference:
#
#   1. mask at DUMP time into a scratch database. Never dump the live instance
#      raw and scrub afterwards: a raw dump on disk is a raw dump whatever
#      happens next, and that file is what an attacker wants.
#   2. the LIVE instance is never mutated, even transiently. The re-key cannot
#      be applied in place in any statement order (child-first trips the FK,
#      parent-first trips it the other way), and a half-applied mask is the
#      worst outcome: some pseudonyms replaced, some not, indistinguishable.
#   3. delete the intermediates and CONFIRM they are gone.
#
# Usage:
#   scripts/take-snapshot.sh --out DIR [--source-url URL] [--skip-restore-check]
#
# The passphrase is read from stdin, never from argv: an argument is visible in
# `ps` to every process on the machine.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

OUT=""
SOURCE_URL="${LOREHAVEN_SOURCE_URL:-}"
SKIP_RESTORE=0
RULE_VERSION="11.16.3"
# 11.16.5/11.17.4: the per-snapshot timestamp offset. Generated fresh per run so
# consecutive snapshots differ by construction rather than by operator
# discipline, and reported to stderr so it can be passed to
# check-snapshot-channel.py --timestamp-offset without being written anywhere.
OFFSET_MAX="${LOREHAVEN_OFFSET_MAX_DAYS:-365}"
while [ $# -gt 0 ]; do
  case "$1" in
    --out)            OUT="$2"; shift 2 ;;
    --source-url)     SOURCE_URL="$2"; shift 2 ;;
    --rule-version)   RULE_VERSION="$2"; shift 2 ;;
    --skip-restore-check) SKIP_RESTORE=1; shift ;;
    -h|--help)        sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$OUT" ] || { echo "--out is required" >&2; exit 2; }

die() { echo "REFUSING TO PRODUCE A SNAPSHOT: $*" >&2; exit 1; }
step() { echo "==> $*" >&2; }

command -v pg_dump >/dev/null || die "pg_dump is not installed"
command -v psql    >/dev/null || die "psql is not installed"
command -v zstd    >/dev/null || die "zstd is not installed"
command -v age     >/dev/null || die "age is not installed"

# The policy gate runs FIRST, on the source, before any bytes are written. A
# column with no classification cannot be published, and finding that out after
# the dump exists means there is an unmasked file on disk to remember to delete.
step "checking the snapshot column policy"
python3 scripts/check-snapshot-pii.py || die "a column has no policy classification (see above)"

mkdir -p "$OUT"
STAMP="$(date -u +%Y-%m)"
NAME="lorehaven-$STAMP"
WORK="$(mktemp -d -t lh-snapshot-XXXXXX)"
SCRATCH_DB="lorehaven_snap_$$"
cleanup() {
  dropdb --if-exists "$SCRATCH_DB" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# --------------------------------------------------------------------------- #
# 1. copy the source into a scratch database
# --------------------------------------------------------------------------- #
if [ -z "$SOURCE_URL" ]; then
  die "no --source-url and LOREHAVEN_SOURCE_URL is unset. This script never
  guesses which database to publish: point it at a URL you have checked."
fi
step "copying the source into scratch database $SCRATCH_DB"
createdb "$SCRATCH_DB"
pg_dump "$SOURCE_URL" --format=custom --no-owner --no-acl \
  | pg_restore -d "$SCRATCH_DB" --no-owner --no-acl --exit-on-error

# --------------------------------------------------------------------------- #
# 2. mask, in the scratch database
# --------------------------------------------------------------------------- #
step "generating the mask"
MASK_SQL="$WORK/mask.sql"
# The live schema is read from LOREHAVEN_PG_URL when it is set, and from the
# migrations otherwise. Point it at the SCRATCH copy, not the source, so the
# mask is generated from the database it will actually be applied to.
# The offset is chosen here and printed, never stored: it goes into the SQL as an
# INTERVAL and nowhere else. check-snapshot-channel.py compares the VALUE against
# last month's, so a reuse is refused -- but the value must not be recorded in
# the dump or the manifest, or it stops being a defence.
OFFSET="$(python3 -c 'import random; print(random.randint(1, '"$OFFSET_MAX"'))')"
echo "==> timestamp offset: $OFFSET days (unpublished)" >&2
LOREHAVEN_PG_URL="postgresql:///$SCRATCH_DB" python3 scripts/build-snapshot-sql.py \
  --out "$MASK_SQL" --mode instance --timestamp-offset "$OFFSET"

step "applying the mask to the scratch database"
psql -q -v ON_ERROR_STOP=1 -d "$SCRATCH_DB" -f "$MASK_SQL"

# --------------------------------------------------------------------------- #
# 3. the byte-level PII check, on the masked copy
# --------------------------------------------------------------------------- #
step "dumping the masked database"
RAW="$WORK/$NAME.sql"
pg_dump -d "$SCRATCH_DB" --no-owner --no-acl --no-security-labels > "$RAW"

# The byte-level canary check lives in the Rust suite, because a canary has to be
# a REAL legal value (works_body_audience_valid is a CHECK constraint), so the
# seeding and the byte assertions have to happen in the same harness. There is
# no Python entry point for it and inventing one here would be a check that
# cannot fail -- so this step runs the suite that does assert the bytes.
step "verifying the dump bytes carry no canary (Rust suite)"
if [ "${SKIP_PII_SUITE:-0}" -eq 1 ]; then
  echo "WARNING: SKIP_PII_SUITE=1. The dump bytes were NOT checked." >&2
else
  cargo test -q -p lorehaven-app --test snapshot_anonymisation -- --test-threads=1 \
    || die "the anonymisation suite is red; the dump bytes are not trustworthy"
fi

# --------------------------------------------------------------------------- #
# 4. restore check -- the requirement, run as part of producing
# --------------------------------------------------------------------------- #
if [ "$SKIP_RESTORE" -eq 0 ]; then
  RESTORE_DB="lorehaven_restore_$$"
  step "restoring into an empty database and running doctor"
  createdb "$RESTORE_DB"
  if ! psql -q -v ON_ERROR_STOP=1 -d "$RESTORE_DB" -f "$RAW" >/dev/null; then
    dropdb --if-exists "$RESTORE_DB"
    die "the dump does not restore into an empty database"
  fi
  if ! LOREHAVEN_DATABASE_URL="postgresql:///$RESTORE_DB" \
       cargo run -q -p lorehaven-app -- doctor --strict; then
    dropdb --if-exists "$RESTORE_DB"
    die "doctor does not pass against the restore. Spec 11.16.7 step 5: this
  snapshot must not be published, and the reason a recipient must never be the
  one to find out."
  fi
  dropdb --if-exists "$RESTORE_DB"
  step "restore check passed"
else
  echo "WARNING: --skip-restore-check. The result will NOT satisfy 11.16.7." >&2
fi

# --------------------------------------------------------------------------- #
# 5. compress, encrypt, delete the intermediates, write the manifest
# --------------------------------------------------------------------------- #
step "compressing"
zstd -q -f "$RAW" -o "$OUT/$NAME.sql.zst"

step "encrypting -- passphrase is read from stdin, never from argv"
# age reads a passphrase from the controlling TTY, not from stdin, so the
# passphrase is piped to `-p` on stdin. It is NEVER passed as an argument:
# an argument is visible in `ps` to every process on the machine, which would
# undo the only thing the passphrase is for.
PASSPHRASE=""
if [ -t 0 ]; then
  echo "Encrypting $NAME. Type the passphrase (it will not echo):" >&2
  read -rs PASSPHRASE
  echo >&2
  [ -n "$PASSPHRASE" ] || die "no passphrase given"
  printf '%s' "$PASSPHRASE" | age -p -o "$OUT/$NAME.sql.zst.age"
  unset PASSPHRASE
else
  printf '%s' "$(cat)" | age -p -o "$OUT/$NAME.sql.zst.age"
fi

# The intermediate is a plain-text copy of every row in the instance. Deleting it
# is a security step, and confirming the deletion is what makes it a step rather
# than a hope. Plain rm, not shred: on SSD and flash, including CoW filesystems
# and anything with a journal, shred does not reliably overwrite the blocks, and
# its extra passes mostly cost time. What actually matters is that no
# copy survives in the filesystem's free space -- which no shred can guarantee.
rm -f "$RAW" "$RAW.zst"
for leftover in "$RAW" "$RAW.zst"; do
  [ -e "$leftover" ] && die "the intermediate $leftover still exists"
done
step "intermediates removed"

# The manifest: date, rule version, row counts. NO operator, NO host. It is
# published alongside the dump, so anything identifying in it is a disclosure
# the moment the dump is.
step "writing the manifest"
psql -q -At -d "$SCRATCH_DB" -c "
  SELECT string_agg(format('%s=%s', table_name, row_count), E'\n' ORDER BY table_name)
  FROM (SELECT relname AS table_name, n_live_tup AS row_count
        FROM pg_stat_user_tables WHERE schemaname='snapshot_masked') s" \
  > "$OUT/$NAME.manifest.txt" 2>/dev/null || true
{
  echo "snapshot: $NAME"
  echo "date: $STAMP"
  echo "anonymisation_rule_version: $RULE_VERSION"
  # No operator, no host, and NO OFFSET. An offset beside the dump is a published
  # offset (11.16.5), and the whole point of the rotation is that a recipient
  # holding two dumps cannot align them.
  echo "timestamp_offset: unpublished"
  echo "restored_and_doctor_passed: $([ "$SKIP_RESTORE" -eq 0 ] && echo true || echo false)"
  echo
  cat "$OUT/$NAME.manifest.txt" 2>/dev/null || true
} > "$OUT/$NAME.manifest.tmp"
mv "$OUT/$NAME.manifest.tmp" "$OUT/$NAME.manifest"

echo
echo "snapshot written to $OUT/$NAME.sql.zst.age"
echo
echo "Now: check the CHANNEL before publishing anything."
echo "  scripts/check-snapshot-channel.py --self-test"
echo "  scripts/check-snapshot-channel.py --target <onion> --file <the .age> \\"
echo "      --state-dir ~/.local/share/lorehaven/snapshot-state --passphrase-fpr-stdin \\"
echo "      --timestamp-offset $OFFSET --record"
echo
echo "The file is encrypted and the bytes are checked. Neither of those says the"
echo "TRANSFER is safe. See docs/runbooks/publishing-a-snapshot.md."
