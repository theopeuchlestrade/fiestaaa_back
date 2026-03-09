ALTER TABLE event_share_tokens
    ADD COLUMN IF NOT EXISTS target_email TEXT,
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

UPDATE event_share_tokens
SET expires_at = COALESCE(expires_at, created_at + INTERVAL '7 days');

ALTER TABLE event_share_tokens
    ALTER COLUMN expires_at SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_share_tokens_target_email
    ON event_share_tokens(target_email);
