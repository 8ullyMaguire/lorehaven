#!/usr/bin/env bash
# Run a frontend tool from any shell.
#
# Why this exists: this checkout may live on a filesystem that cannot execute
# the `node_modules/.bin` shims (an sshfs mount, for instance), and `npm run`
# fails there with "command not found". Invoking the same entry points through
# `node` works everywhere, and does exactly what the npm scripts do.
#
# Usage: scripts/fe.sh build | dev | test [args...] | check
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

case "${1:-build}" in
  build)
    exec node ./node_modules/vite/bin/vite.js build
    ;;
  dev)
    exec node ./node_modules/vite/bin/vite.js dev
    ;;
  test)
    exec node ./node_modules/vitest/vitest.mjs run "${@:2}"
    ;;
  check)
    exec node ./node_modules/svelte-check/bin/svelte-check --tsconfig ./tsconfig.json
    ;;
  e2e)
    # Journeys in a real browser against the built release binary.
    # Requires: npx playwright install chromium (once).
    #
    # The suite does not rebuild anything. `serve-scratch.sh` reuses whatever
    # binary is at `$LOREHAVEN_BIN`, and the binary embeds `frontend/dist` via
    # rust_embed -- so a frontend change is invisible to the journeys until
    # BOTH halves are rebuilt. A comment saying "requires a built binary" was
    # not a guarantee: it let a run serve a binary predating the change and
    # report a CSS token as 65px when the source said 68.6px, which reads as an
    # analytics bug and is not one.
    #
    # So refuse a stale binary instead of trusting the caller. Compare the
    # binary's mtime against the frontend sources and dist; if either is newer,
    # the embedded assets cannot be current, and a red run would be a lie.
    bin="${LOREHAVEN_BIN:-$HOME/.cargo-target/lorehaven/release/lorehaven}"
    if [ ! -x "$bin" ]; then
      echo "fe.sh: no binary at $bin -- run: cargo build --release --bin lorehaven" >&2
      exit 1
    fi
    stale=""
    for src in src dist; do
      [ -e "$here/$src" ] || continue
      newest="$(find "$here/$src" -newer "$bin" -print -quit 2>/dev/null || true)"
      if [ -n "$newest" ]; then
        stale="$stale $src"
      fi
    done
    if [ -n "$stale" ]; then
      echo "fe.sh: $bin is OLDER than frontend:$stale" >&2
      echo "fe.sh: the binary embeds frontend/dist, so the journeys would test" >&2
      echo "fe.sh: assets that no longer exist. Rebuild both halves:" >&2
      echo "fe.sh:   ./scripts/fe.sh build && cargo build --release --bin lorehaven" >&2
      exit 1
    fi
    LOREHAVEN_BIN="$bin" \
      exec node ./node_modules/@playwright/test/cli.js test "${@:2}"
    ;;
  *)
    echo "usage: $0 build|dev|test|check" >&2
    exit 2
    ;;
esac
