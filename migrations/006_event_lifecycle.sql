ALTER TABLE events
    ADD COLUMN IF NOT EXISTS timezone TEXT NOT NULL DEFAULT 'Europe/Paris',
    ADD COLUMN IF NOT EXISTS starts_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS effective_ends_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS purge_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS deletion_reason TEXT;

CREATE OR REPLACE FUNCTION fiestaaa_sync_event_instants()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.starts_at := (NEW.date_event + NEW.start_time) AT TIME ZONE NEW.timezone;
    IF NEW.end_date IS NOT NULL AND NEW.end_time IS NOT NULL THEN
        NEW.effective_ends_at := (NEW.end_date + NEW.end_time) AT TIME ZONE NEW.timezone;
    ELSE
        NEW.effective_ends_at := ((NEW.date_event + 1)::date::timestamp) AT TIME ZONE NEW.timezone;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS events_sync_instants ON events;
CREATE TRIGGER events_sync_instants
BEFORE INSERT OR UPDATE OF date_event, start_time, end_date, end_time, timezone
ON events
FOR EACH ROW
EXECUTE FUNCTION fiestaaa_sync_event_instants();

UPDATE events
SET timezone = COALESCE(NULLIF(timezone, ''), 'Europe/Paris');

ALTER TABLE events
    ALTER COLUMN starts_at SET NOT NULL,
    ALTER COLUMN effective_ends_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'events_effective_schedule_check'
    ) THEN
        ALTER TABLE events ADD CONSTRAINT events_effective_schedule_check
            CHECK (effective_ends_at >= starts_at);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'events_deletion_reason_check'
    ) THEN
        ALTER TABLE events ADD CONSTRAINT events_deletion_reason_check
            CHECK (deletion_reason IS NULL OR deletion_reason IN ('owner', 'retention'));
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'events_purge_state_check'
    ) THEN
        ALTER TABLE events ADD CONSTRAINT events_purge_state_check
            CHECK (
                (deleted_at IS NULL AND purge_at IS NULL AND deletion_reason IS NULL)
                OR (deleted_at IS NOT NULL AND purge_at IS NOT NULL AND deletion_reason IS NOT NULL)
            );
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS idx_events_active_start
    ON events(starts_at, event_id)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_events_purge
    ON events(purge_at)
    WHERE deleted_at IS NOT NULL;
