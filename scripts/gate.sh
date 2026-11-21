#!/usr/bin/env bash
# Quality gate (guidelines section 0). Run from the workspace root. Tests need the development PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v protoc >/dev/null 2>&1; then
    printf '%s\n' 'protoc not found on PATH; install the Protocol Buffers compiler before running the gate.' >&2
    exit 1
fi
. scripts/lib.sh
load_env
export SQLX_OFFLINE=true
# The vendored htmx file must match its recorded hash.
if ! (cd crates/ops/assets && grep -v '^#' htmx.sha256 | sha256sum --check --quiet -); then
    printf '%s\n' 'crates/ops/assets/htmx.min.js does not match htmx.sha256' >&2
    exit 1
fi
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
# Process tests need the publisher, gateway, and launcher executables.
cargo build --bin publisher --bin gateway --bin launcher
cargo test --workspace
