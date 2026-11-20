#!/usr/bin/env bash
# Quality gate (guidelines section 0). Run from the workspace root. Tests need the development PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v protoc >/dev/null 2>&1; then
    printf '%s\n' 'protoc not found on PATH; install the Protocol Buffers compiler before running the gate.' >&2
    exit 1
fi
# An already set DATABASE_URL (compose provides it inside the container) is not overwritten by the .env value.
preset_database_url="${DATABASE_URL:-}"
if [ -f .env ]; then set -a; . ./.env; set +a; fi
if [ -n "$preset_database_url" ]; then export DATABASE_URL="$preset_database_url"; fi
export SQLX_OFFLINE=true
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
