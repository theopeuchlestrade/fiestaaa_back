-- Shared playlist metadata for events

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS playlist_url TEXT,
    ADD COLUMN IF NOT EXISTS playlist_provider TEXT;

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT events_playlist_provider_check
            CHECK (
                (playlist_url IS NULL AND playlist_provider IS NULL)
                OR (
                    playlist_url IS NOT NULL
                    AND length(trim(playlist_url)) > 0
                    AND playlist_provider IN ('spotify', 'apple_music', 'deezer')
                )
            );
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;
