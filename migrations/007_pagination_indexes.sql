CREATE INDEX IF NOT EXISTS idx_events_owner_event_id
    ON events (owner_user_id, event_id)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_invitations_user_event_id
    ON invitations (user_id, event_id DESC);

CREATE INDEX IF NOT EXISTS idx_friendships_user_a_created
    ON friendships (user_a, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_friendships_user_b_created
    ON friendships (user_b, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_friend_requests_participants_id
    ON friend_requests (sender_id, receiver_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_event_expenses_event_id
    ON event_expenses (event_id, expense_id DESC);

CREATE INDEX IF NOT EXISTS idx_carpools_event_id
    ON carpools (event_id, carpool_id);
