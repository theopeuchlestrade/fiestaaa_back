CREATE TABLE IF NOT EXISTS pending_registrations (
    email TEXT PRIMARY KEY,
    password_hash TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    verification_token UUID NOT NULL UNIQUE,
    verification_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_pending_registrations_verification_token
    ON pending_registrations(verification_token);
