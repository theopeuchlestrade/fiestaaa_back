ALTER TABLE pending_registrations
    ADD COLUMN IF NOT EXISTS verification_token_ciphertext BYTEA;
