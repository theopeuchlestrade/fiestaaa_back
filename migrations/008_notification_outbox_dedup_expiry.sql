ALTER TABLE notification_outbox
    ADD COLUMN IF NOT EXISTS dedup_expires_at TIMESTAMPTZ;

UPDATE notification_outbox
SET dedup_expires_at = created_at + INTERVAL '5 minutes'
WHERE dedup_key IS NOT NULL
  AND dedup_expires_at IS NULL;

CREATE INDEX IF NOT EXISTS notification_outbox_retention
    ON notification_outbox(status, COALESCE(sent_at, created_at))
    WHERE status IN ('sent', 'dead');
