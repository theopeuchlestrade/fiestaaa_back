DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'carpools_event_driver_unique'
    ) THEN
        ALTER TABLE carpools
            ADD CONSTRAINT carpools_event_driver_unique UNIQUE (event_id, driver_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'carpools_id_event_unique'
    ) THEN
        ALTER TABLE carpools
            ADD CONSTRAINT carpools_id_event_unique UNIQUE (carpool_id, event_id);
    END IF;
END
$$;

ALTER TABLE carpool_passengers
    ADD COLUMN IF NOT EXISTS event_id BIGINT;

UPDATE carpool_passengers cp
SET event_id = c.event_id
FROM carpools c
WHERE c.carpool_id = cp.carpool_id;

ALTER TABLE carpool_passengers
    ALTER COLUMN event_id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'carpool_passengers_carpool_event_fk'
    ) THEN
        ALTER TABLE carpool_passengers
            ADD CONSTRAINT carpool_passengers_carpool_event_fk
            FOREIGN KEY (carpool_id, event_id)
            REFERENCES carpools(carpool_id, event_id)
            ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'carpool_passengers_event_user_unique'
    ) THEN
        ALTER TABLE carpool_passengers
            ADD CONSTRAINT carpool_passengers_event_user_unique UNIQUE (event_id, user_id);
    END IF;
END
$$;

CREATE INDEX IF NOT EXISTS idx_carpool_passengers_event
    ON carpool_passengers(event_id);
