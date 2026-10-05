#!/usr/bin/env bash
# Create (or repair) the PostgreSQL scratch container the two-engine test runs need.
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
#
# The URL is printed to stdout for `eval`; nothing is secret beyond the well-known
# local test password.

set -euo pipefail

NAME=lh-pg-test
IMAGE=postgres:15-alpine
PORT=55433
USER_NAME=lorehaven
PASSWORD=lorehaven
DB=postgres

# 16 GB of shared memory is what the host actually has; 4 GB leaves room for everything
# else on the box while letting ~128 concurrent 32 MB segments allocate.
SHM_SIZE=${LOREHAVEN_PG_SHM:-4g}

recreate=0
if [ "${1:-}" = "--recreate" ]; then
  recreate=1
fi

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
echo "export LOREHAVEN_TEST_PG_URL='${url}'" >&2
echo "$url"
