-- Configurable event menu modules

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS enabled_features TEXT[] NOT NULL
        DEFAULT ARRAY['carpools', 'polls', 'items'];

UPDATE events
SET enabled_features = ARRAY_REMOVE(
    ARRAY[
        'carpools',
        'polls',
        'items',
        CASE
            WHEN playlist_provider IS NOT NULL
                AND playlist_url IS NOT NULL
                AND length(trim(playlist_url)) > 0
                THEN 'playlist'
            ELSE NULL
        END,
        CASE
            WHEN payment_provider_id IS NOT NULL
                AND payment_identifier IS NOT NULL
                AND length(trim(payment_identifier)) > 0
                THEN 'payment'
            ELSE NULL
        END
    ],
    NULL
)
WHERE enabled_features = ARRAY['carpools', 'polls', 'items'];

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT events_enabled_features_check
            CHECK (
                enabled_features <@ ARRAY['carpools', 'polls', 'items', 'playlist', 'payment']::TEXT[]
            );
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;
