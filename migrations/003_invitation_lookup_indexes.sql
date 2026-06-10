CREATE INDEX IF NOT EXISTS idx_invitations_user_status_event
    ON invitations(user_id, status, event_id);

CREATE INDEX IF NOT EXISTS idx_invitations_event_status_user
    ON invitations(event_id, status, user_id);
