#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
service=$1
shift
. scripts/lib.sh
case $service in server|publisher|ops) load_env ;; esac
if [[ $service == ops ]]; then strip_ops_secrets; fi
build_unless_prebuilt --bin "$service"
exec "$(resolve_binary "$service")" --config "$(config_for "$service")" "$@"
