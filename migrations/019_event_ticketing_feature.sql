-- Allow the optional ticketing module in events.enabled_features

UPDATE events
SET enabled_features = array_append(enabled_features, 'ticketing')
WHERE NOT (enabled_features @> ARRAY['ticketing']::TEXT[]);

ALTER TABLE events
    DROP CONSTRAINT IF EXISTS events_enabled_features_check;

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT events_enabled_features_check
            CHECK (
                enabled_features <@ ARRAY[
                    'carpools',
                    'polls',
                    'items',
                    'ticketing',
                    'playlist',
                    'payment',
                    'expenses'
                ]::TEXT[]
            );
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;
