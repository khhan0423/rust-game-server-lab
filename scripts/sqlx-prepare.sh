#!/usr/bin/env bash
# Describe all SQL queries against a disposable schema in development PostgreSQL.
set -euo pipefail
cd "$(dirname "$0")/.."
set -a
. ./.env
set +a

scratch=sqlx_check
psql_dev() {
    docker compose --env-file .env -f dev/compose.yaml exec -T postgres \
        psql -X -v ON_ERROR_STOP=1 -U postgres "$@"
}

psql_dev -d postgres -c "DROP DATABASE IF EXISTS $scratch" -c "CREATE DATABASE $scratch"
for migration in migrations/*/*.sql; do
    psql_dev -d "$scratch" < "$migration"
done

database_url=${DATABASE_URL%%\?*}
export DATABASE_URL="${database_url%/*}/$scratch${DATABASE_URL#"$database_url"}"
export SQLX_OFFLINE=false

case "${1:-}" in
    '') cargo sqlx prepare --workspace -- --all-targets ;;
    --db-only)
        # Use when another workspace member is temporarily unbuildable.
        (cd crates/db && cargo sqlx prepare -- --all-targets)
        mkdir -p .sqlx
        mv crates/db/.sqlx/query-*.json .sqlx/
        rmdir crates/db/.sqlx
        ;;
    *) printf 'Usage: %s [--db-only]\n' "$0" >&2; exit 2 ;;
esac
