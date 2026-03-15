-- Allow the optional ticketing module in events.enabled_features

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
