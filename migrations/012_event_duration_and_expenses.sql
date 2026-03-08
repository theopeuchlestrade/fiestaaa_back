-- Event duration support and shared expenses feature

ALTER TABLE events
    ADD COLUMN IF NOT EXISTS end_date DATE,
    ADD COLUMN IF NOT EXISTS end_time TIME;

UPDATE events
SET end_date = NULL,
    end_time = NULL
WHERE end_date IS NOT NULL
  AND end_time IS NULL;

UPDATE events
SET end_date = NULL,
    end_time = NULL
WHERE end_date IS NULL
  AND end_time IS NOT NULL;

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT events_end_datetime_check
            CHECK (
                (end_date IS NULL AND end_time IS NULL)
                OR (
                    end_date IS NOT NULL
                    AND end_time IS NOT NULL
                    AND (end_date, end_time) >= (date_event, start_time)
                )
            );
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;

ALTER TABLE events
    DROP CONSTRAINT IF EXISTS events_enabled_features_check;

DO $$
BEGIN
    BEGIN
        ALTER TABLE events
            ADD CONSTRAINT events_enabled_features_check
            CHECK (
                enabled_features <@ ARRAY['carpools', 'polls', 'items', 'playlist', 'payment', 'expenses']::TEXT[]
            );
    EXCEPTION
        WHEN duplicate_object THEN
            NULL;
    END;
END
$$;

CREATE TABLE IF NOT EXISTS event_expenses (
    expense_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    paid_by_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    amount_cents BIGINT NOT NULL CHECK (amount_cents > 0),
    note TEXT,
    expense_date TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_event_expenses_event ON event_expenses(event_id, expense_date DESC);

CREATE TABLE IF NOT EXISTS event_expense_participants (
    expense_id BIGINT NOT NULL REFERENCES event_expenses(expense_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    share_weight INT NOT NULL DEFAULT 1 CHECK (share_weight > 0),
    PRIMARY KEY (expense_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_event_expense_participants_user ON event_expense_participants(user_id);
