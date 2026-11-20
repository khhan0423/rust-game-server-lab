#!/usr/bin/env bash
# Quality gate (guidelines section 0). Run from the workspace root. Tests need the development PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")/.."
# An already set DATABASE_URL (compose provides it inside the container) is not overwritten by the .env value.
preset_database_url="${DATABASE_URL:-}"
if [ -f .env ]; then set -a; . ./.env; set +a; fi
if [ -n "$preset_database_url" ]; then export DATABASE_URL="$preset_database_url"; fi
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
