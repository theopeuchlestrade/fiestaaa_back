CREATE TABLE IF NOT EXISTS oauth_identities (
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, provider_subject)
);

CREATE INDEX IF NOT EXISTS idx_oauth_identities_user_id
    ON oauth_identities(user_id);
