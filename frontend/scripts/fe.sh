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
  *)
    echo "usage: $0 build|dev|test|check" >&2
    exit 2
    ;;
esac
