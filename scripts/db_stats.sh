#!/usr/bin/env bash
set -euo pipefail

# Quick database summary (users, events, invitations, devices, check-ins)
# Usage: cd ~/apps/fiestaaa && ./scripts/db_stats.sh

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"

# Load .env if present (for DATABASE_URL or POSTGRES_*) while ignoring invalid lines
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

# Compose command (Compose V2 required)
compose_cmd="docker compose"
if ! docker compose version >/dev/null 2>&1; then
  echo "docker compose not found. Install the Compose V2 plugin (docker-compose-plugin) and rerun." >&2
  exit 1
fi

# Build a URL if DATABASE_URL is not provided
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
    # Fallback: run psql from the db container
    DB_USER="${POSTGRES_USER:-postgres}"
    DB_NAME="${POSTGRES_DB:-postgres}"
    $compose_cmd exec -T db psql -U "$DB_USER" -d "$DB_NAME" "$@"
  fi
}

md_table() {
  local title="$1"
  local header="$2"
  local query="$3"

  echo -e "\n### $title\n"
  echo "| ${header//|/ | } |"
  local underline=""
  IFS='|' read -ra cols <<<"$header"
  for _ in "${cols[@]}"; do underline+="| --- "; done
  underline+="|"
  echo "$underline"

  # psql output in unaligned mode with '|' separator, then pipe again to format Markdown
  run_psql -A -F '|' -t -c "$query" | sed 's/^/| /; s/|/ | /g; s/$/ |/'
}

md_table "Summary" "users_total|events_total|invitations_accepted|invitations_waiting|invitations_declined|checkins_total|devices_active" "
SELECT
  (SELECT count(*) FROM users) AS users_total,
  (SELECT count(*) FROM events) AS events_total,
  (SELECT count(*) FROM invitations WHERE status = 'Accepted') AS invitations_accepted,
  (SELECT count(*) FROM invitations WHERE status = 'Waiting') AS invitations_waiting,
  (SELECT count(*) FROM invitations WHERE status = 'Declined') AS invitations_declined,
  (SELECT count(*) FROM event_checkins) AS checkins_total,
  (SELECT count(*) FROM user_devices WHERE disabled_at IS NULL) AS devices_active;
"

md_table "Invitations by status" "status|invitations" "
SELECT status, count(*) AS invitations
FROM invitations
GROUP BY status
ORDER BY status;
"

md_table "Active devices by platform" "platform|active_devices" "
SELECT platform, count(*) AS active_devices
FROM user_devices
WHERE disabled_at IS NULL
GROUP BY platform
ORDER BY platform;
"

md_table "New users (last 14 days)" "day|new_users" "
SELECT date_trunc('day', created_at)::date AS day, count(*) AS new_users
FROM users
GROUP BY day
ORDER BY day DESC
LIMIT 14;
"
