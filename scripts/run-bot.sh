#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export SQLX_OFFLINE=true
if [[ ${RUN_PREBUILT:-0} != 1 ]]; then cargo build --bin bot; fi
binary="${CARGO_TARGET_DIR:-target}/debug/bot"
if [[ -f $binary.exe ]]; then binary=$binary.exe; fi
exec "$binary" --config "${BOT_CONFIG:-dev/config/bot.toml}" "$@"
