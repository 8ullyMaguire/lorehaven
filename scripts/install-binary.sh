#!/usr/bin/env bash
# Install this checkout's binary into the path the service runs, and say which
# checkout it is.
#
# The trap this exists for: rust/darkhold and rust/darkhold-draw are two
# checkouts of the same crate, they share ONE run path
# (~/.cargo-target/darkhold/release/darkhold) and ONE user unit, and each has its
# own uncommitted work. Any session that builds and restarts therefore replaces
# whatever the other session deployed, silently. The install is now:
#
#   * from this checkout's own target/ (the .cargo/config.toml already pins it),
#   * with the binary it replaces kept in ~/backups,
#   * with a sidecar next to the binary naming the checkout, commit and dirty
#     state that produced it, so "what is running" is answerable after the fact.
#
# Usage: scripts/install-binary.sh [--no-restart]
set -euo pipefail

CHECKOUT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_DIR="${HOME}/.cargo-target/darkhold/release"
RUN_BIN="${RUN_DIR}/darkhold"
BUILT="${CHECKOUT}/target/release/darkhold"

cd "$CHECKOUT"

echo "== building ${CHECKOUT}"
cargo build --release -p darkhold-app --bin darkhold

if [[ ! -x "$BUILT" ]]; then
  echo "build produced no binary at ${BUILT}" >&2
  exit 1
fi

COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
DIRTY="$(git status --porcelain 2>/dev/null | head -1 | wc -l | tr -d ' ')"
STAMP="$(date -Iseconds)"

mkdir -p "$RUN_DIR" "${HOME}/backups"

if [[ -f "$RUN_BIN" ]]; then
  # A running binary cannot be overwritten in place: `cp` fails with "Text file
  # busy". Stopping first is not enough on its own -- the old file is still the
  # unit's ExecStart, so it is kept rather than deleted.
  KEEP="${HOME}/backups/darkhold-replaced-$(date +%Y%m%d-%H%M%S).bin"
  cp "$RUN_BIN" "$KEEP"
  echo "== kept the binary being replaced: ${KEEP}"
  echo "   it came from: $(cat "${RUN_DIR}/darkhold.built-from" 2>/dev/null || echo '(no provenance sidecar)')"
fi

systemctl --user stop darkhold.service || true
cp "$BUILT" "$RUN_BIN"
printf 'checkout=%s\ncommit=%s\ndirty_files=%s\nbuilt_at=%s\n' \
  "$CHECKOUT" "$COMMIT" "$DIRTY" "$STAMP" > "${RUN_DIR}/darkhold.built-from"

if [[ "${1:-}" != "--no-restart" ]]; then
  systemctl --user start darkhold.service
  for _ in $(seq 1 20); do
    if curl -fsS -o /dev/null "http://127.0.0.1:8081/health/live"; then
      echo "== installed ${COMMIT}${DIRTY:+ (dirty: ${DIRTY} files)} from ${CHECKOUT}"
      echo "   running: $(systemctl --user is-active darkhold.service)"
      exit 0
    fi
    sleep 1
  done
  echo "service did not become healthy; journalctl --user -u darkhold -n 40" >&2
  exit 1
fi
