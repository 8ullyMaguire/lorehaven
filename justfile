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
    cd {{frontend_dir}} && npm run build

# Build everything, in the order the deploy uses: bundle first, then the binary
# that embeds it.
all: frontend build

# --- checks -----------------------------------------------------------------

fmt:
    cargo fmt --all

lint:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test --workspace

# Frontend unit tests.
test-frontend:
    cd {{frontend_dir}} && npm test

# Everything CI runs.
check: fmt lint test test-frontend
    cargo fmt --all -- --check

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
    cd {{frontend_dir}} && npm run dev

# --- housekeeping -----------------------------------------------------------

# Start from an empty development database.
reset-dev:
    rm -rf ./data
    cargo run -- migrate
    cargo run -- seed --development

clean:
    cargo clean
    rm -rf {{frontend_dir}}/dist
