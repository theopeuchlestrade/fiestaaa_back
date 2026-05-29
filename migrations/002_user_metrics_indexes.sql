CREATE INDEX IF NOT EXISTS idx_users_created_at ON users(created_at);
CREATE INDEX IF NOT EXISTS idx_pending_registrations_created_at
    ON pending_registrations(created_at);

CREATE INDEX IF NOT EXISTS idx_oauth_identities_last_login_at
    ON oauth_identities(last_login_at);
CREATE INDEX IF NOT EXISTS idx_user_devices_active_last_seen
    ON user_devices(last_seen)
    WHERE disabled_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_friend_requests_created_at
    ON friend_requests(created_at);
CREATE INDEX IF NOT EXISTS idx_friend_requests_responded_at
    ON friend_requests(responded_at)
    WHERE responded_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_share_tokens_created_at
    ON event_share_tokens(created_at);
CREATE INDEX IF NOT EXISTS idx_event_share_tokens_used_at
    ON event_share_tokens(used_at)
    WHERE used_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_checkins_generated_at
    ON event_checkins(generated_at);
CREATE INDEX IF NOT EXISTS idx_event_checkins_scanned_at
    ON event_checkins(scanned_at)
    WHERE scanned_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_polls_created_at ON event_polls(created_at);
CREATE INDEX IF NOT EXISTS idx_event_poll_votes_created_at
    ON event_poll_votes(created_at);

CREATE INDEX IF NOT EXISTS idx_carpools_created_at ON carpools(created_at);
CREATE INDEX IF NOT EXISTS idx_carpool_passengers_joined_at
    ON carpool_passengers(joined_at);

CREATE INDEX IF NOT EXISTS idx_event_expenses_created_at
    ON event_expenses(created_at);

CREATE INDEX IF NOT EXISTS idx_friendships_created_at ON friendships(created_at);
