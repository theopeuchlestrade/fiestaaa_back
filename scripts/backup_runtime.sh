#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"
BACKUP_ROOT="${POSTGRES_BACKUP_ROOT:-${ROOT_DIR}/data/backups}"
RETENTION_DAYS="${POSTGRES_BACKUP_RETENTION_DAYS:-14}"
INCLUDE_FILES="${BACKUP_INCLUDE_FILES:-true}"
INCLUDE_SECRETS="${BACKUP_INCLUDE_SECRETS:-false}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"

is_truthy() {
  case "$1" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

is_falsy() {
  case "$1" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

if [[ -f "$ENV_FILE" ]]; then
  set -a
  while IFS= read -r line; do
    [[ -z "$line" || "$line" =~ ^# ]] && continue
    if [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      export "${line%%=*}=${line#*=}"
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

mkdir -p "${BACKUP_ROOT}/postgres" "${BACKUP_ROOT}/files"
chmod 700 "${BACKUP_ROOT}" "${BACKUP_ROOT}/postgres" "${BACKUP_ROOT}/files"

dump_tmp="${BACKUP_ROOT}/postgres/fiestaaa_${TIMESTAMP}.dump.tmp"
dump_path="${BACKUP_ROOT}/postgres/fiestaaa_${TIMESTAMP}.dump"

if command -v pg_dump >/dev/null 2>&1; then
  pg_dump --format=custom --no-owner --no-privileges --file "$dump_tmp" "$DATABASE_URL"
else
  compose_cmd="docker compose"
  if ! docker compose version >/dev/null 2>&1; then
    echo "pg_dump and docker compose are both unavailable" >&2
    exit 1
  fi
  db_user="${POSTGRES_USER:-postgres}"
  db_name="${POSTGRES_DB:-postgres}"
  $compose_cmd exec -T db pg_dump --format=custom --no-owner --no-privileges -U "$db_user" -d "$db_name" > "$dump_tmp"
fi

mv "$dump_tmp" "$dump_path"
chmod 600 "$dump_path"

file_items=()
if ! is_falsy "$INCLUDE_FILES"; then
  [[ -d "${ROOT_DIR}/data/uploads" ]] && file_items+=("data/uploads")
  [[ -f "${ROOT_DIR}/traefik/letsencrypt/acme.json" ]] && file_items+=("traefik/letsencrypt/acme.json")

  if is_truthy "$INCLUDE_SECRETS"; then
    [[ -f "${ROOT_DIR}/.env" ]] && file_items+=(".env")
    [[ -f "${ROOT_DIR}/data/service-account.json" ]] && file_items+=("data/service-account.json")
  fi
fi

if [[ "${#file_items[@]}" -gt 0 ]]; then
  files_tmp="${BACKUP_ROOT}/files/fiestaaa_files_${TIMESTAMP}.tar.gz.tmp"
  files_path="${BACKUP_ROOT}/files/fiestaaa_files_${TIMESTAMP}.tar.gz"
  tar -C "$ROOT_DIR" -czf "$files_tmp" "${file_items[@]}"
  mv "$files_tmp" "$files_path"
  chmod 600 "$files_path"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$dump_path" > "${dump_path}.sha256"
elif command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$dump_path" > "${dump_path}.sha256"
fi
[[ -f "${dump_path}.sha256" ]] && chmod 600 "${dump_path}.sha256"

find "${BACKUP_ROOT}/postgres" -type f -name 'fiestaaa_*.dump*' -mtime "+${RETENTION_DAYS}" -delete
find "${BACKUP_ROOT}/files" -type f -name 'fiestaaa_files_*.tar.gz' -mtime "+${RETENTION_DAYS}" -delete

echo "Backup complete: ${dump_path}"
