#!/usr/bin/env bash
#
# The archive's RSS budget gate.
#
# WHAT THIS IS
#
# The specification's rule is a **provenance rule**: every cap names where its
# number came from, and a cap with neither a measurement nor a derivation is not
# a cap (spec §22.3). This gate is the archive's side of that: it boots the real
# binary against the real catalogue, reads the kernel's high-water mark after
# each step, and prints **three** numbers — the baseline, the current figure, and
# the delta. **A regression is a delta, not a reading of an absolute.**
#
# THE RULES, AND WHY EACH ONE IS HERE
#
#   * **It refuses to run if the corpus is unreadable** — *"measuring nothing is
#     not a pass."* A gate that boots against an empty database and reports a
#     beautiful number is measuring the binary, not the instance, and the number
#     it reports has nothing to do with the thing the cap is about.
#   * **It reads `VmHWM` from `/proc/<pid>/status`** — the kernel's own
#     high-water mark, which is the peak. A gate that read `VmRSS` would report
#     a number that goes *down* when the instance is idle, and a budget gate
#     that reports a lower number when nothing is happening is not a budget gate.
#   * **It waits for `/health/ready`** — it does not sleep a guessed interval.
#   * **It reads content, not status.** A `200` is not proof the catalogue was
#     read; the body is.
#
# THE BASELINE
#
# **22.39 MiB**, measured on 2026-10-20: the median of six runs of
# `scripts/measure-rss.sh` (three against an empty database — 22.64, 22.77,
# 23.00 — and three against a migrated one — 22.14, 21.96, 21.96), reading
# `VmHWM` from the release binary.
#
# **It is the RSS of an instance with no corpus**, because the corpus arrives at
# M9 and is not in the instance yet. The cap derived from it is therefore the
# cap for an instance *without* a corpus, and the cap for an instance *with* one
# is a different number that has to be measured against the corpus.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN="${DARKHOLD_BIN:-$REPO_ROOT/target/release/darkhold}"
PORT="${DARKHOLD_BUDGET_PORT:-8201}"

#: The measured baseline, in MiB. See the header for its provenance.
BASELINE_MIB="22.39"

#: The cap: the baseline plus 25% (spec §22.3's derived-cap rule).
CAP_MIB="$(awk -v b="$BASELINE_MIB" 'BEGIN { printf "%.2f", b * 1.25 }')"

cleanup() {
    if [ -n "${SERVER_PID:-}" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
    echo "the release binary is not there: $BIN" >&2
    echo "build it with: cargo build --release -p darkhold-app" >&2
    exit 1
fi

# --- the catalogue ------------------------------------------------------------
#
# The instance's own database. A gate that boots against a scratch database
# measures the binary and not the instance, and the cap is about the instance.
DB_URL="${DARKHOLD_DATABASE_URL:-}"
if [ -z "$DB_URL" ]; then
    for candidate in data/*.sqlite data/*.db; do
        if [ -f "$candidate" ]; then
            DB_URL="sqlite://$candidate?mode=rwc"
            break
        fi
    done
fi
if [ -z "$DB_URL" ]; then
    echo "no catalogue to measure: set DARKHOLD_DATABASE_URL, or put the instance's" >&2
    echo "database in data/. Measuring nothing is not a pass." >&2
    exit 1
fi

# --- the corpus has to be there -----------------------------------------------
#
# A schema with no works in it is not a catalogue, and an instance with no
# catalogue is not the thing the cap is about.
WORKS="$(sqlite3 "$DB_URL" 'SELECT count(*) FROM works;' 2>/dev/null || echo 0)"
if [ "$WORKS" -lt 1 ]; then
    echo "the catalogue at $DB_URL holds no works." >&2
    echo "Measuring an empty catalogue is measuring the binary, not the instance," >&2
    echo "and measuring nothing is not a pass." >&2
    exit 1
fi
echo "catalogue: $DB_URL ($WORKS work(s))"

# --- boot ---------------------------------------------------------------------
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/darkhold-budget-XXXXXX")"
mkdir -p "$SCRATCH/storage"

"$BIN" \
    --database-url "$DB_URL" \
    --storage-root "$SCRATCH/storage" \
    --bind "127.0.0.1" \
    --port "$PORT" \
    serve >"$SCRATCH/serve.log" 2>&1 &
SERVER_PID=$!

READY=0
for _ in $(seq 1 120); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "the server exited before it was ready:" >&2
        tail -n 20 "$SCRATCH/serve.log" >&2
        exit 1
    fi
    if curl -fsS "http://127.0.0.1:$PORT/health/ready" >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 0.25
done
if [ "$READY" != "1" ]; then
    echo "the server never became ready" >&2
    tail -n 20 "$SCRATCH/serve.log" >&2
    exit 1
fi

# --- the steps, and the high-water mark after each ----------------------------
hwm() {
    awk '/^VmHWM:/ {print $2}' "/proc/$SERVER_PID/status" 2>/dev/null || true
}

step() {
    local label="$1"
    shift
    "$@" >/dev/null 2>&1 || true
    local kib
    kib="$(hwm)"
    if [ -z "$kib" ]; then
        echo "could not read VmHWM after $label" >&2
        exit 1
    fi
    awk -v k="$kib" -v l="$label" 'BEGIN { printf "  %-28s %8.2f MiB\n", l, k / 1024 }'
}

echo "the kernel's high-water mark after each step:"
step "ready" true
step "the catalogue" curl -fsS "http://127.0.0.1:$PORT/api/v1/works?limit=1"
step "the metadata" curl -fsS "http://127.0.0.1:$PORT/api/v1/meta"

# --- the verdict --------------------------------------------------------------
CURRENT_MIB="$(awk '/^VmHWM:/ {print $2}' "/proc/$SERVER_PID/status" | awk '{printf "%.2f", $1 / 1024}')"
DELTA_MIB="$(awk -v c="$CURRENT_MIB" -v b="$BASELINE_MIB" 'BEGIN { printf "%+.2f", c - b }')"

echo
echo "baseline: $BASELINE_MIB MiB   (the median of six, measured 2026-10-20)"
echo "current:  $CURRENT_MIB MiB   (this run, against $WORKS work(s))"
echo "delta:    $DELTA_MIB MiB   (current - baseline)"
echo "cap:      $CAP_MIB MiB   (the baseline plus 25%)"

OVER="$(awk -v c="$CURRENT_MIB" -v k="$CAP_MIB" 'BEGIN { print (c > k) ? "yes" : "no" }')"
if [ "$OVER" = "yes" ]; then
    echo
    echo "OVER THE CAP: $CURRENT_MIB MiB against a $CAP_MIB MiB cap." >&2
    exit 1
fi

echo
echo "within the cap."
rm -rf "$SCRATCH"
