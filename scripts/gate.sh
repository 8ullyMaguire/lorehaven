#!/usr/bin/env bash
#
# The gate runner.
#
# WHY THIS EXISTS
#
# `just` is not installed on this host, and the gates were being run by hand, one at a
# time, with the command typed from memory. A gate that is run from memory is a gate
# that is sometimes run with the wrong flags, and the estate's rule is that a gate is a
# command, not an intention.
#
# It also exists because of a specific failure the spec records (§23.2):
#
#     cargo clippy ... 2>/dev/null | grep -cE '^(warning|error)'
#
# returned a confident **zero** from an empty stdout, while the tree held 30 warnings --
# three of them real defects, including a test that had never run. `rustc`, `clippy` and
# `cargo test` report on **stderr**. So:
#
#   * Every gate writes to a log file, and the gate's **exit code** is what is read.
#   * The log is printed in full on failure, because "it failed" without the reason is
#     a gate that costs more time than it saves.
#   * **No gate is ever piped through `grep -c`.** A tally of "test result:" lines is
#     blind to a target that never ran, which is the failure this script exists to make
#     impossible.
#
# USAGE
#
#   scripts/gate.sh fmt                     cargo fmt --all --check
#   scripts/gate.sh lint                    cargo clippy, warnings are errors
#   scripts/gate.sh build                   cargo build --all-targets
#   scripts/gate.sh test [crate] [filter]   cargo test on SQLite
#   scripts/gate.sh pg [crate] [filter]     the same, on PostgreSQL
#   scripts/gate.sh doc                     the doctests, standalone
#   scripts/gate.sh frontend                svelte-check and vitest
#   scripts/gate.sh e2e <suite>             one Playwright suite at a time
#   scripts/gate.sh dialects                the dialect-parity gates
#   scripts/gate.sh repo                    the reachability probe and the split-spec check
#   scripts/gate.sh budget                  the archive's RSS budget
#   scripts/gate.sh taste-budget            the taste engine's RSS budget (M13-09)
#   scripts/gate.sh all                     fmt, lint, build, test, doc, frontend, dialects
#
# Every log lands in `target/gate/<name>.log`.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE_DIR="$REPO_ROOT/target/gate"

# The PostgreSQL admin URL. The default is the one the plan's own gate commands use; the
# environment wins, so a developer with the cluster on another port does not have to
# edit this file.
PG_URL="${DARKHOLD_TEST_PG_URL:-postgres://postgres:postgres@127.0.0.1:55433/postgres}"

# Where the test harness puts its scratch directories.
#
# **`/tmp` is a 16 GB tmpfs on the development host, and the harness leaks its scratch
# directories** — a workspace run leaves thousands of `darkhold-*` directories, because
# `TestDb` has no `Drop` and `std::env::temp_dir()` is not swept (the harness says so in
# its own comment). A full run needs several gigabytes of them, so on a 16 GB tmpfs the
# suite fills the filesystem and the failures arrive as **"database or disk is full"** —
# which reads like a storage problem and is a tmpfs problem.
#
# So the scratch lives on the real filesystem, and the stale directories are swept
# before the run. **A gate that fails because it ran out of room is not a gate that
# measured anything.**
SCRATCH_ROOT="${DARKHOLD_SCRATCH:-$REPO_ROOT/target/scratch}"
export TMPDIR="$SCRATCH_ROOT"
mkdir -p "$SCRATCH_ROOT"

sweep_scratch() {
    # Only this harness's directories, and only ones that are clearly abandoned.
    # A run in flight has a live process holding a handle, so the sweep is for
    # directories left by a run that is over.
    local dir
    for dir in "$SCRATCH_ROOT"/darkhold-*; do
        [ -d "$dir" ] || continue
        rm -rf "$dir" 2>/dev/null || true
    done
}

# How many lines of a failed gate's log to print. Enough to see the assertion, not so
# much that the failure scrolls off the top of the terminal.
TAIL_LINES="${DARKHOLD_GATE_TAIL:-40}"

# The port the URL points at, parsed out of it rather than written down twice.
pg_port() {
    local rest="${PG_URL#*://}"
    rest="${rest#*@}"
    local hostport="${rest%%/*}"
    case "$hostport" in
        *:*) printf '%s' "${hostport##*:}" ;;
        *) printf '%s' "5432" ;;
    esac
}

usage() {
    sed -n '3,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# Run one gate: write its output to `target/gate/<name>.log`, and on failure print the
# log's tail and the exit code.
#
# The exit code is the verdict. There is no tally anywhere in this function, on purpose.
run_gate() {
    local name="$1"
    shift
    local log="$GATE_DIR/$name.log"
    mkdir -p "$GATE_DIR"

    printf '==> %s\n' "$name"
    local rc=0
    set +e
    "$@" > "$log" 2>&1
    rc=$?
    set -e

    if [ "$rc" -ne 0 ]; then
        printf 'FAILED: %s (exit %s)\n' "$name" "$rc"
        printf -- '--- the last %s lines of %s ---\n' "$TAIL_LINES" "$log"
        tail -n "$TAIL_LINES" "$log"
        printf -- '--- end of %s ---\n' "$log"
        exit "$rc"
    fi
    printf 'ok: %s\n' "$name"
}

# The PostgreSQL gate REFUSES to run when the cluster is not there.
#
# It does not skip, and it does not exit 0. A gate that quietly does nothing is worse
# than no gate at all, because the record says it passed (spec §23.2: "A result is
# green only when the exit code is zero", and "If a gate has never once failed, suspect
# the gate before you trust it").
require_postgres() {
    local port
    port="$(pg_port)"
    if ! (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
        printf 'PostgreSQL is not reachable on 127.0.0.1:%s.\n' "$port" >&2
        printf 'Start it with `bash scripts/pg-test-db.sh` (or `--local` on a host with no\n' >&2
        printf 'container runtime), and re-run this gate. The PostgreSQL arm is NOT being\n' >&2
        printf 'skipped: it is a required gate for anything touching SQL.\n' >&2
        exit 3
    fi
}

# `cargo test` for the SQLite arm. `crate` and `filter` are both optional.
run_tests() {
    local crate="${1:-}"
    local filter="${2:-}"
    if [ -n "$crate" ] && [ -n "$filter" ]; then
        cargo test -p "$crate" "$filter" --no-fail-fast
    elif [ -n "$crate" ]; then
        cargo test -p "$crate" --no-fail-fast
    elif [ -n "$filter" ]; then
        cargo test "$filter" --no-fail-fast
    else
        cargo test --workspace --no-fail-fast
    fi
}

# The five PostgreSQL dialect-parity gates, plus the gate that finds test files which
# claim dual-backend coverage and then pin SQLite.
#
# `check-sqlite-migration-syntax.py` is a sixth dialect gate and it runs here too: it
# checks the SQLite side of the same parity, and leaving it out of `all` would be a gap
# in the one gate that is supposed to close gaps.
run_dialects() {
    local failed=0
    local script
    for script in \
        scripts/check-pg-arm-uses-sqlite-pool.py \
        scripts/check-pg-backend-arm-placeholders.py \
        scripts/check-pg-subquery-alias.py \
        scripts/check-pg-uuid-casts.py \
        scripts/check-uncast-pg-placeholders.py \
        scripts/check-sqlite-migration-syntax.py \
        scripts/find-single-backend-suites.py
    do
        if ! python3 "$script"; then
            failed=1
        fi
    done
    return "$failed"
}

# The repository-level checks: the things that are true of the TREE rather than
# of one crate.
#
#   * the reachability probe -- every table has a reader and a writer, or is
#     listed in the known-gaps file with the reason and the pinning test;
#   * the split-spec checker -- the specification's split is in step with its
#     master, and every file's heading number matches its filename.
run_repo() {
    python3 scripts/reachability.py
    python3 scripts/check-split-spec.py
}

# The frontend gate: the type check and the unit tests.
run_frontend() {
    cd "$REPO_ROOT/frontend"
    node ./node_modules/svelte-check/bin/svelte-check --tsconfig ./tsconfig.json
    node ./node_modules/vitest/vitest.mjs run
}

# One Playwright suite at a time, against the built release binary.
run_e2e() {
    local suite="${1:-}"
    if [ -z "$suite" ]; then
        printf 'e2e needs a suite name. The suites are:\n' >&2
        ls "$REPO_ROOT/frontend/e2e"/*.spec.ts 2>/dev/null | sed 's#.*/##; s#\.spec\.ts$##' | sed 's/^/  /' >&2
        exit 2
    fi
    cd "$REPO_ROOT/frontend"
    node ./node_modules/@playwright/test/cli.js test "$suite"
}

cmd="${1:-}"
[ $# -gt 0 ] && shift

case "$cmd" in
    fmt)
        run_gate fmt cargo fmt --all --check
        ;;
    lint)
        # `-- -D warnings` is what makes the exit code mean something: `cargo clippy`
        # exits 0 with warnings present, which is the tally trap the spec records.
        run_gate lint cargo clippy --workspace --all-targets -- -D warnings
        ;;
    build)
        run_gate build cargo build --all-targets
        ;;
    test)
        sweep_scratch
        run_gate test run_tests "$@"
        ;;
    pg)
        require_postgres
        sweep_scratch
        DARKHOLD_TEST_PG_URL="$PG_URL" run_gate pg run_tests "$@"
        ;;
    doc)
        # Standalone: the doctests are never mixed with target flags.
        run_gate doc cargo test --workspace --doc
        ;;
    frontend)
        run_gate frontend run_frontend
        ;;
    e2e)
        run_gate e2e run_e2e "$@"
        ;;
    dialects)
        run_gate dialects run_dialects
        ;;
    repo)
        run_gate repo run_repo
        ;;
    budget)
        # The archive's RSS budget (T-M1-06). It boots the real binary against the
        # real catalogue, reads the kernel's high-water mark after each step, and
        # prints the baseline, the current figure and the delta. It REFUSES to run
        # when the catalogue is unreadable: measuring nothing is not a pass.
        run_gate budget bash scripts/budget.sh
        ;;
    taste-budget)
        # NOT IMPLEMENTED YET — the taste engine's budget gate is T-M13-09.
        #
        # It exits 2, and it does not exit 0. A gate that did not run must never report
        # success, or the record says the budget was checked when it was not.
        printf "taste-budget: not implemented (T-M13-09, the taste engine's RSS gate)\n" >&2
        exit 2
        ;;
    all)
        run_gate fmt cargo fmt --all --check
        run_gate lint cargo clippy --workspace --all-targets -- -D warnings
        run_gate build cargo build --all-targets
        run_gate test run_tests
        run_gate doc cargo test --workspace --doc
        run_gate frontend run_frontend
        run_gate dialects run_dialects
        run_gate repo run_repo
        ;;
    --help | -h | help | "")
        usage
        exit 0
        ;;
    *)
        printf 'unknown gate: %s\n\n' "$cmd" >&2
        usage >&2
        exit 2
        ;;
esac
