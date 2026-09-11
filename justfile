# Lorehaven development recipes.
#
# `just <recipe>` — see https://github.com/casey/just
# Every recipe is a thin wrapper over a command that is also documented in
# docs/tutorial/, so nothing here is the only way to do something.

set shell := ["bash", "-euo", "pipefail", "-c"]

frontend_dir := "frontend"
binary := "target/debug/lorehaven"

default:
    @just --list

# --- setup ------------------------------------------------------------------

# Install frontend dependencies.
setup:
    cd {{frontend_dir}} && npm ci

# --- build ------------------------------------------------------------------

# Build the backend.
build:
    cargo build

# Build the release binary.
build-release:
    cargo build --release

# Build the frontend bundle.
frontend:
    {{frontend_dir}}/scripts/fe.sh build

# Build everything, in the order the deploy uses: bundle first, then the binary
# that embeds it.
all: frontend build

# --- checks -----------------------------------------------------------------
#
# The frontend recipes reach their tools through `frontend/scripts/fe.sh` rather
# than `npm run`. This checkout may sit on a filesystem that cannot execute the
# `node_modules/.bin` shims, where `npm run build` fails with "vite: command not
# found" while the same entry point invoked through `node` works — see the header
# of that script. Routing every recipe the same way means `just check` here means
# what it means in CI.

fmt:
    cargo fmt --all

lint:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --workspace

# Frontend unit tests.
test-frontend:
    {{frontend_dir}}/scripts/fe.sh test

# Frontend type and accessibility check.
#
# This belongs in `check`: it is the step CI runs and the one that catches the
# most, and it used to be missing here. Its absence is why 65 pre-existing
# `svelte-check` failures — 44 of them the same message against 44 files — sat
# unnoticed in a tree whose owner ran `just check` and saw green.
check-frontend:
    {{frontend_dir}}/scripts/fe.sh check

# Everything CI runs.
check: fmt lint test test-frontend check-frontend
    cargo fmt --all -- --check

# Drive every library endpoint against a live PostgreSQL server.
#
# Deliberately not part of `check`: it needs a running server and a scratch
# database, and it drops that database's `public` schema. It is here because the
# tests run on SQLite, and SQLite accepts SQL that PostgreSQL refuses — four
# defects in milestone 8's own code were found only by this. See the script's
# header for the server it expects and for what `DATABASE_URL` and `PSQL` override.
journey-postgres:
    ./scripts/postgres-journey.sh

# --- run --------------------------------------------------------------------

serve:
    cargo run -- serve

# Serve with the frontend read from disk, so a Vite dev server or a rebuilding
# bundle is picked up without recompiling Rust.
serve-dev:
    cargo run -- --assets-dir {{frontend_dir}}/dist serve

migrate:
    cargo run -- migrate

migrate-status:
    cargo run -- migrate --status

seed:
    cargo run -- seed --development

doctor:
    cargo run -- doctor

# Start the Vite dev server (proxies /api and /health to the Rust server).
dev-frontend:
    {{frontend_dir}}/scripts/fe.sh dev

# --- housekeeping -----------------------------------------------------------

# Start from an empty development database.
reset-dev:
    rm -rf ./data
    cargo run -- migrate
    cargo run -- seed --development

clean:
    cargo clean
    rm -rf {{frontend_dir}}/dist
