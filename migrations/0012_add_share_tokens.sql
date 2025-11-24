CREATE TABLE IF NOT EXISTS event_share_tokens (
    token UUID PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    created_by_email TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    used_at TIMESTAMPTZ,
    used_by_email TEXT
);

CREATE INDEX IF NOT EXISTS idx_event_share_tokens_event ON event_share_tokens(event_id);
