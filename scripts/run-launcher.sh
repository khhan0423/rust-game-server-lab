#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib.sh
build_unless_prebuilt --bin launcher --bin publisher --bin server --bin gateway --bin ops
exec "$(resolve_binary launcher)" "$@"
