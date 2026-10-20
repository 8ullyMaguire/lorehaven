#!/usr/bin/env bash
#
# Measure the archive's peak RSS.
#
# WHY THIS IS A SCRIPT AND NOT A NUMBER
#
# The specification's rule is a **provenance rule**: every cap names where its
# number came from, and a cap with neither a measurement nor a derivation is not
# a cap (spec §22.3). The archive's RSS cap is *derived* — the measured baseline
# plus a stated margin — and the measured baseline is what this script produces.
#
# So the script has to be trustworthy in three specific ways, each of which is a
# recorded failure somewhere:
#
#   * **It waits for `/health/ready`.** It does not sleep a guessed interval,
#     because a guess that is too short measures a process that has not finished
#     starting, and a guess that is too long measures an idle process.
#   * **It reads `VmHWM` from `/proc/<pid>/status`.** `VmHWM` is the kernel's own
#     high-water mark: the peak, not the current resident set. A script that read
#     `VmRSS` would report a *lowered* number after the peak, and a budget gate
#     that reads the current RSS never fails.
#   * **It measures the RELEASE binary.** The debug build allocates differently,
#     and a budget measured against the wrong binary is a budget for a program
#     nobody ships.
#
# USAGE
#
#   scripts/measure-rss.sh                 # once, against an empty scratch database
#   scripts/measure-rss.sh --migrated      # once, against a migrated scratch database
#   scripts/measure-rss.sh --runs 3        # three times, and the median
#
# It prints the peak in MiB, one number, and exits 0. A failure to measure is a
# failure: it exits non-zero and says why, because **"measuring nothing is not a
# pass."**

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN="${DARKHOLD_BIN:-$REPO_ROOT/target/release/darkhold}"
PORT="${DARKHOLD_MEASURE_PORT:-8199}"
RUNS=1
MIGRATED=0
SCRATCH=""

cleanup() {
    if [ -n "$SCRATCH" ]; then
        rm -rf "$SCRATCH"
    fi
}
trap cleanup EXIT

while [ $# -gt 0 ]; do
    case "$1" in
        --migrated) MIGRATED=1; shift ;;
        --runs) RUNS="$2"; shift 2 ;;
        *) echo "usage: $0 [--migrated] [--runs N]" >&2; exit 2 ;;
    esac
done

if [ ! -x "$BIN" ]; then
    echo "the release binary is not there: $BIN" >&2
    echo "build it with: cargo build --release -p darkhold-app" >&2
    exit 1
fi

# The port has to be free, or the measurement is of whatever was already on it.
if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then
    echo "port $PORT is already in use; set DARKHOLD_MEASURE_PORT to a free one" >&2
    exit 1
fi

# One measurement: start, wait for readiness, read the high-water mark, stop.
measure_once() {
    local label="$1"
    local dir
    dir="$(mktemp -d "${TMPDIR:-/tmp}/darkhold-rss-XXXXXX")"
    SCRATCH="$dir"
    mkdir -p "$dir/storage"
    local db="sqlite://$dir/darkhold.sqlite?mode=rwc"

    # A migrated database, when asked for: the archive's own `migrate` command,
    # so the measurement is of a database the instance would actually have.
    if [ "$MIGRATED" = "1" ]; then
        "$BIN" --database-url "$db" --storage-root "$dir/storage" migrate >/dev/null 2>&1 || {
            echo "the scratch database could not be migrated" >&2
            return 1
        }
    fi

    "$BIN" \
        --database-url "$db" \
        --storage-root "$dir/storage" \
        --bind "127.0.0.1" \
        --port "$PORT" \
        serve >"$dir/serve.log" 2>&1 &
    local pid=$!

    # Wait for readiness, by asking. Never by sleeping a guessed interval.
    local ready=0
    for _ in $(seq 1 120); do
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "the server exited before it was ready:" >&2
            tail -n 20 "$dir/serve.log" >&2
            return 1
        fi
        if curl -fsS "http://127.0.0.1:$PORT/health/ready" >/dev/null 2>&1; then
            ready=1
            break
        fi
        sleep 0.25
    done
    if [ "$ready" != "1" ]; then
        echo "the server never became ready" >&2
        tail -n 20 "$dir/serve.log" >&2
        kill "$pid" 2>/dev/null || true
        return 1
    fi

    # The kernel's high-water mark: the peak, not the current resident set.
    local hwm_kib
    hwm_kib="$(awk '/^VmHWM:/ {print $2}' "/proc/$pid/status" 2>/dev/null || true)"
    if [ -z "$hwm_kib" ]; then
        echo "could not read VmHWM for pid $pid" >&2
        kill "$pid" 2>/dev/null || true
        return 1
    fi

    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true

    # KiB to MiB, to two decimals.
    awk -v kib="$hwm_kib" 'BEGIN { printf "%.2f\n", kib / 1024 }'
    SCRATCH=""
    rm -rf "$dir"
}

measurements=()
for _ in $(seq 1 "$RUNS"); do
    value="$(measure_once "run")"
    measurements+=("$value")
    printf '%s\n' "$value"
done

if [ "${#measurements[@]}" -gt 1 ]; then
    median="$(printf '%s\n' "${measurements[@]}" | sort -n | awk '{a[NR]=$1} END { if (NR%2) print a[(NR+1)/2]; else printf "%.2f\n", (a[NR/2]+a[NR/2+1])/2 }')"
    printf 'median: %s MiB\n' "$median"
fi
