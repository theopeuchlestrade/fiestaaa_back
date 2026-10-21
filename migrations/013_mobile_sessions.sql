-- Native refresh credentials are separate from short-lived API access tokens.
CREATE TABLE mobile_sessions (
 id UUID PRIMARY KEY,
 user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 session_version BIGINT NOT NULL,
 token_hash TEXT NOT NULL UNIQUE,
 previous_token_hash TEXT UNIQUE,
 previous_valid_until TIMESTAMPTZ,
 token_ciphertext BYTEA NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '90 days'
);
CREATE INDEX mobile_sessions_expiry ON mobile_sessions(expires_at);
