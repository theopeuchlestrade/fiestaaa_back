-- Event-bound polls for quick participant feedback

CREATE TABLE IF NOT EXISTS event_polls (
    poll_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    allow_multiple BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_event_polls_event ON event_polls(event_id);
CREATE INDEX IF NOT EXISTS idx_event_polls_expires_at ON event_polls(expires_at);

CREATE TABLE IF NOT EXISTS event_poll_options (
    option_id BIGSERIAL PRIMARY KEY,
    poll_id BIGINT NOT NULL REFERENCES event_polls(poll_id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    position INT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_event_poll_options_poll ON event_poll_options(poll_id);

CREATE TABLE IF NOT EXISTS event_poll_votes (
    poll_id BIGINT NOT NULL REFERENCES event_polls(poll_id) ON DELETE CASCADE,
    option_id BIGINT NOT NULL REFERENCES event_poll_options(option_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (poll_id, user_id, option_id)
);

CREATE INDEX IF NOT EXISTS idx_event_poll_votes_poll ON event_poll_votes(poll_id);
CREATE INDEX IF NOT EXISTS idx_event_poll_votes_user ON event_poll_votes(user_id);
