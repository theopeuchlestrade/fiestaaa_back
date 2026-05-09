#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"
SCHEMA_FILE="${ROOT_DIR}/migrations/001_initial_schema.sql"

if [[ ! -f "$SCHEMA_FILE" ]]; then
  echo "Consolidated schema not found: $SCHEMA_FILE" >&2
  exit 1
fi

if [[ -f "$ENV_FILE" ]]; then
  set -a
  while IFS= read -r line; do
    [[ -z "$line" || "$line" =~ ^# ]] && continue
    if [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      key="${line%%=*}"
      value="${line#*=}"
      export "${key}=${value}"
    fi
  done < "$ENV_FILE"
  set +a
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  DB_USER="${POSTGRES_USER:-postgres}"
  DB_PASS="${POSTGRES_PASSWORD:-postgres}"
  DB_HOST="${POSTGRES_HOST:-db}"
  DB_PORT="${POSTGRES_PORT:-5432}"
  DB_NAME="${POSTGRES_DB:-postgres}"
  DATABASE_URL="postgres://${DB_USER}:${DB_PASS}@${DB_HOST}:${DB_PORT}/${DB_NAME}"
fi

run_psql() {
  if command -v psql >/dev/null 2>&1; then
    psql "$DATABASE_URL" "$@"
  else
    if ! docker compose version >/dev/null 2>&1; then
      echo "psql and docker compose were not found. Install one of them and rerun." >&2
      exit 1
    fi
    compose_cmd="docker compose"
    DB_USER="${POSTGRES_USER:-postgres}"
    DB_NAME="${POSTGRES_DB:-postgres}"
    $compose_cmd exec -T db psql -U "$DB_USER" -d "$DB_NAME" "$@"
  fi
}

echo "Full database rebuild from ${SCHEMA_FILE}"

run_psql -v ON_ERROR_STOP=1 <<'SQL'
DROP SCHEMA IF EXISTS public CASCADE;
CREATE SCHEMA public;
SQL

run_psql -v ON_ERROR_STOP=1 -f "$SCHEMA_FILE"

echo "Database rebuilt successfully."
