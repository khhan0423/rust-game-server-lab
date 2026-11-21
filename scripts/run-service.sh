#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
service=$1
default_command=$2
shift 2
set -a
. ./.env
set +a
if [[ $service == ops ]]; then
    unset DATABASE_URL LOGIN_DB_URL GAME_DB_URL POSTGRES_PASSWORD PUBLISHER_GAME_API_KEY
fi
if [[ $# == 0 && -n $default_command ]]; then
    set -- "$default_command"
fi
export SQLX_OFFLINE=true
cargo build --bin "$service"
binary="${CARGO_TARGET_DIR:-target}/debug/$service"
if [[ -f $binary.exe ]]; then binary=$binary.exe; fi
exec "$binary" --config "dev/config/$service.toml" "$@"
