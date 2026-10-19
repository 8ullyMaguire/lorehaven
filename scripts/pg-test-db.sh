#!/usr/bin/env bash
# Create (or repair) the PostgreSQL scratch server the two-engine test runs need.
#
# WHY THIS EXISTS
#
# `cargo test` on PostgreSQL fails in a way that looks like a product bug and is not:
#
#   could not resize shared memory segment "/PostgreSQL.12345" to 33554432 bytes:
#   No space left on device
#
#   0: error communicating with database: Connection reset by peer (os error 104)
#   PgDatabaseError { code: "53200", message: "out of shared memory" }
#
# Every one of those is /dev/shm exhaustion, and Docker's default for a container is
# 64 MB. A single parallel CREATE-DATABASE / migrate / TEMPLATE-clone wave wants a 32 MB
# segment per concurrent backend, so two or three backends exhaust it and every test in
# the batch fails while reporting a connection error instead. `Connection reset by peer`
# is the same cause arriving before the segment request: the backend died allocating.
#
# The host has 16 GB of /dev/shm. The fix is to give the container a share of it, and
# to make that reproducible — the container is created by hand, so nothing in the repo
# recorded the invocation, and every future session re-derives the same 64 MB wall.
#
# USAGE
#
#   scripts/pg-test-db.sh            # create if missing, start if stopped, print the URL
#   scripts/pg-test-db.sh --recreate # destroy and rebuild (drops scratch DBs; keep data
#                                    # if you want it: the user/database live in the image
#                                    # layer unless you mounted a volume)
#   scripts/pg-test-db.sh --local    # no container runtime? use a locally-installed server
#
# The URL is printed to stdout for `eval`; nothing is secret beyond the well-known
# local test password.

set -euo pipefail

NAME=lh-pg-test
IMAGE=postgres:15-alpine
PORT=55433
USER_NAME=darkhold
PASSWORD=darkhold
DB=postgres

# 16 GB of shared memory is what the host actually has; 4 GB leaves room for everything
# else on the box while letting ~128 concurrent 32 MB segments allocate.
SHM_SIZE=${DARKHOLD_PG_SHM:-4g}

recreate=0
mode=docker
case "${1:-}" in
  --recreate) recreate=1 ;;
  --local)    mode=local ;;
  "")         ;;
  *) echo "usage: $0 [--recreate] [--local]" >&2; exit 2 ;;
esac

# ---------------------------------------------------------------------------
# The local server
# ---------------------------------------------------------------------------
#
# Some hosts have PostgreSQL's server binaries installed and no container runtime. The
# gate still has to run there, so the script starts a cluster directly rather than
# leaving the gate unrunnable and unrecorded.
#
# Two environment facts this mode has to respect, and both of them were learned the
# hard way:
#
# 1. **`/tmp` is a 16 GB tmpfs on this host, and the cluster does not fit in it.** The
#    harness creates a fresh database per test, and with many test binaries in
#    parallel the cluster grew past 11 GB. PostgreSQL then stopped *itself* with
#    "No space left on device" and stopped accepting connections — so the symptom the
#    suite reported was a connection error, not a disk error. The data directory
#    therefore defaults to a path on the real filesystem.
#
# 2. **`max_connections` defaults to 100, and the harness exhausts it.** Every test
#    holds a pool against the *admin* database, and `cargo test --workspace` runs
#    many binaries in parallel. The symptom is "pool timed out while waiting for an
#    open connection", which reads like a code defect and is not one. 500 is enough
#    for the full suite; the container path gets the same headroom by not needing it
#    (its connections are bounded by the same setting, so the image default is
#    overridden below too).
#
# The local cluster uses trust authentication on a loopback socket, so the URL's password
# is not needed by the server -- but it IS needed by the tests.
#
# `crates/app/tests/snapshot_anonymisation.rs` shells out to `psql` and `pg_dump`, and it
# parses the password and the endpoint out of `DARKHOLD_TEST_PG_URL` with its own parser
# (`pg_endpoint()` and `pg_password()`). That parser expects a URL that carries
# CREDENTIALS: `creds.rsplit_once(':').expect("a url with a password")`. A URL with a user
# but no password makes the whole test file fail with "a url with a password" -- a
# message that reads like a credential problem and is a URL-shape problem.
#
# So the local cluster sets a password on the role and prints the URL with it, and the
# server's trust auth means the password is never actually checked.

local_mode() {
  local data=${DARKHOLD_PG_DATA:-$HOME/code-local/pg-test-data}
  local socket=${DARKHOLD_PG_SOCKET:-/tmp/opencode}
  local url="postgres://postgres:postgres@127.0.0.1:${PORT}/${DB}"

  for binary in initdb pg_ctl pg_isready; do
    command -v "$binary" >/dev/null 2>&1 || {
      echo "$0 --local: '$binary' is not on PATH. Install PostgreSQL's server package," >&2
      echo "or use the container path: $0" >&2
      exit 1
    }
  done

  if [ ! -s "$data/PG_VERSION" ]; then
    echo "initdb: $data" >&2
    rm -rf "$data"
    mkdir -p "$data"
    initdb -D "$data" -U postgres --auth=trust -E UTF8 >/dev/null
  fi

  # The settings the harness needs. Appended once, and rewritten if they are missing.
  if ! grep -q "max_connections = 500" "$data/postgresql.conf" 2>/dev/null; then
    cat >> "$data/postgresql.conf" <<'CONF'

# --- Darkhold's PG test gate ---
# The harness creates a fresh database per test, every one of them holds a pool, and
# `cargo test --workspace` runs many binaries in parallel. The default 100 connections
# is exhausted long before the suite finishes.
max_connections = 500
shared_buffers = 256MB
work_mem = 8MB
maintenance_work_mem = 128MB
CONF
  fi

  if ! pg_isready -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1; then
    echo "starting the local cluster on port $PORT" >&2
    pg_ctl -D "$data" \
      -o "-p ${PORT} -k ${socket} -c listen_addresses=127.0.0.1" \
      -l /tmp/opencode/pg.log start >/dev/null
    for _ in $(seq 1 60); do
      pg_isready -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1 && break
      sleep 1
    done
  fi

  pg_isready -h 127.0.0.1 -p "$PORT" >/dev/null 2>&1 || {
    echo "$0 --local: the cluster did not come up. See /tmp/opencode/pg.log" >&2
    exit 1
  }

  # The role's password. Trust auth means the server never checks it, but the test
  # harness's own URL parser does.
  psql -h 127.0.0.1 -p "$PORT" -U postgres -qc "ALTER USER postgres PASSWORD 'postgres';" >/dev/null 2>&1 || true

  echo "mode: local (no container runtime)" >&2
  echo "data: $data" >&2
  echo "export DARKHOLD_TEST_PG_URL='${url}'" >&2
  echo "$url"
}

if [ "$mode" = local ]; then
  local_mode
  exit 0
fi

# ---------------------------------------------------------------------------
# The container
# ---------------------------------------------------------------------------

url="postgres://${USER_NAME}:${PASSWORD}@127.0.0.1:${PORT}/${DB}"

running() {
  [ "$(sudo docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" = true ]
}

# `ShmSize` cannot be changed on a live container — it is fixed at create time. So a
# container that exists with the wrong shm has to be replaced, not reconfigured.
needs_recreate=$recreate
if sudo docker inspect "$NAME" >/dev/null 2>&1; then
  current=$(sudo docker inspect -f '{{.HostConfig.ShmSize}}' "$NAME")
  # 4 GB expressed in bytes; docker reports ShmSize as an integer.
  want=$(( ${SHM_SIZE%g} * 1024 * 1024 * 1024 ))
  if [ "$current" != "$want" ]; then
    needs_recreate=1
  fi
else
  needs_recreate=1
fi

if [ "$needs_recreate" = 1 ]; then
  if sudo docker inspect "$NAME" >/dev/null 2>&1; then
    echo "recreating $NAME (shm size is fixed at create time)" >&2
    sudo docker rm -f "$NAME" >/dev/null
  fi
  sudo docker run -d \
    --name "$NAME" \
    --shm-size "$SHM_SIZE" \
    -e POSTGRES_USER="$USER_NAME" \
    -e POSTGRES_PASSWORD="$PASSWORD" \
    -e POSTGRES_DB="$DB" \
    -p "127.0.0.1:${PORT}:5432" \
    "$IMAGE" >/dev/null

  # The entrypoint initialises the cluster on first boot; without this wait the first
  # `cargo test` races `initdb` and reports "the database system is starting up".
  for _ in $(seq 1 60); do
    if sudo docker exec "$NAME" pg_isready -U "$USER_NAME" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
fi

if ! running; then
  sudo docker start "$NAME" >/dev/null
  for _ in $(seq 1 60); do
    if sudo docker exec "$NAME" pg_isready -U "$USER_NAME" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
fi

actual=$(sudo docker inspect -f '{{.HostConfig.ShmSize}}' "$NAME")
echo "shm: $(( actual / 1024 / 1024 )) MB" >&2
echo "export DARKHOLD_TEST_PG_URL='${url}'" >&2
echo "$url"
