#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export SQLX_OFFLINE=true
if [[ ${RUN_PREBUILT:-0} != 1 ]]; then cargo build --bin gateway; fi
binary="${CARGO_TARGET_DIR:-target}/debug/gateway"
if [[ -f $binary.exe ]]; then binary=$binary.exe; fi
exec "$binary" --config "${GATEWAY_CONFIG:-dev/config/gateway.toml}" "$@"
