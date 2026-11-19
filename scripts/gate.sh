#!/usr/bin/env bash
# 품질 게이트 (가이드 0장). 워크스페이스 루트에서 실행한다. 테스트에는 개발용 PostgreSQL이 필요하다.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -f .env ]; then set -a; . ./.env; set +a; fi
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
