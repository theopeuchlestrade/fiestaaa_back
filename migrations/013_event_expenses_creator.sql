ALTER TABLE event_expenses
    ADD COLUMN IF NOT EXISTS created_by_user_id BIGINT REFERENCES users(id) ON DELETE CASCADE;

UPDATE event_expenses
SET created_by_user_id = paid_by_user_id
WHERE created_by_user_id IS NULL;

ALTER TABLE event_expenses
    ALTER COLUMN created_by_user_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_expenses_creator
    ON event_expenses(created_by_user_id);
