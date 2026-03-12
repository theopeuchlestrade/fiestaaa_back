ALTER TABLE events
    ADD COLUMN IF NOT EXISTS invitation_deadline DATE;

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT invitation_deadline_before_event
            CHECK (invitation_deadline IS NULL OR invitation_deadline <= date_event);
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;
