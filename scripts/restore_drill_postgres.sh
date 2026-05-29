#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_ROOT="${POSTGRES_BACKUP_ROOT:-${ROOT_DIR}/data/backups}"
CONTAINER_NAME="${RESTORE_DRILL_CONTAINER:-fiestaaa_restore_drill}"
POSTGRES_IMAGE="${RESTORE_DRILL_POSTGRES_IMAGE:-postgres:18}"
RESTORE_DB="${RESTORE_DRILL_DATABASE:-fiestaaa_restore}"

dump_path="${1:-}"
if [[ -z "$dump_path" ]]; then
  dump_path="$(find "${BACKUP_ROOT}/postgres" -type f -name 'fiestaaa_*.dump' | sort | tail -n 1)"
fi

if [[ -z "$dump_path" || ! -f "$dump_path" ]]; then
  echo "No Postgres backup dump found in ${BACKUP_ROOT}/postgres" >&2
  exit 1
fi

cleanup() {
  docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
docker run -d \
  --name "$CONTAINER_NAME" \
  -e POSTGRES_PASSWORD=restore-drill \
  -e POSTGRES_DB="$RESTORE_DB" \
  "$POSTGRES_IMAGE" >/dev/null

for _ in $(seq 1 30); do
  if docker exec "$CONTAINER_NAME" pg_isready -U postgres -d "$RESTORE_DB" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

docker exec "$CONTAINER_NAME" pg_isready -U postgres -d "$RESTORE_DB" >/dev/null
docker exec -i "$CONTAINER_NAME" pg_restore \
  --no-owner \
  --no-privileges \
  -U postgres \
  -d "$RESTORE_DB" < "$dump_path"

table_count="$(
  docker exec "$CONTAINER_NAME" psql -U postgres -d "$RESTORE_DB" -tAc \
    "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public';"
)"

if [[ "$table_count" -lt 1 ]]; then
  echo "Restore drill failed: no public tables restored from ${dump_path}" >&2
  exit 1
fi

echo "Restore drill OK: ${dump_path} restored with ${table_count} public tables"
