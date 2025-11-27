-- Add user handle for login/search (case-insensitive unique)
ALTER TABLE users ADD COLUMN IF NOT EXISTS handle TEXT;

UPDATE users
SET handle = CONCAT('fiestaaa-', id)
WHERE handle IS NULL OR handle = '';

ALTER TABLE users
    ALTER COLUMN handle SET NOT NULL;

-- Enforce case-insensitive uniqueness on handle
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'users_handle_unique_idx'
    ) THEN
        CREATE UNIQUE INDEX users_handle_unique_idx ON users (lower(handle));
    END IF;
END$$;
