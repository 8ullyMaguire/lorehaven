#!/usr/bin/env bash
#
# Serve a scratch instance for the Playwright suite and seed what the UI
# legitimately cannot create (forum categories are admin/seed-only).
#
# Playwright starts this as `webServer.command`, waits for /health/ready,
# runs the journeys, and terminates this script; the trap shuts the server
# down with it. Scratch state lives under frontend/test-results/scratch,
# which Playwright clears between runs.
set -uo pipefail
cd "$(dirname "$0")/.."

rm -rf test-results/scratch
mkdir -p test-results/scratch/storage

BIN=${LOREHAVEN_BIN:-$HOME/.cargo-target/lorehaven/release/lorehaven}
PORT=${LOREHAVEN_E2E_PORT:-8173}
SCRATCH="$(pwd)/test-results/scratch"

"$BIN" \
  --storage-root "$SCRATCH/storage" \
  --database-url "sqlite://$SCRATCH/lorehaven.db?mode=rwc" \
  --port "$PORT" \
  serve --with-worker &
SRV=$!
trap 'kill "$SRV" 2>/dev/null' EXIT

for _ in $(seq 1 120); do
  curl -fsS "http://127.0.0.1:$PORT/health/ready" 2>/dev/null | grep -q '"ready"' && break
  sleep 0.5
done
curl -fsS "http://127.0.0.1:$PORT/health/ready" 2>/dev/null | grep -q '"ready"' || {
  echo "e2e server did not come up" >&2
  exit 1
}

python3 - <<'PYEOF'
import os
import sqlite3

db = sqlite3.connect(os.path.join(os.getcwd(), "test-results/scratch/lorehaven.db"))
db.execute(
    "INSERT OR IGNORE INTO forum_categories (id, name, position, min_trust) "
    "VALUES ('11111111-1111-1111-1111-111111111111', 'General discussion', 0, 0)"
)
db.commit()
print("e2e scratch seeded")
PYEOF

wait "$SRV"
