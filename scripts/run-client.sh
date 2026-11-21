#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --bin client -- --config "${CLIENT_CONFIG:-dev/config/client.toml}" "$@"
