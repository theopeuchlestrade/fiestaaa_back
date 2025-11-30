-- QR Code Check-in System
-- Allows event organizers to verify attendee presence via QR code scanning

-- Table to track QR codes for event check-ins
CREATE TABLE IF NOT EXISTS event_checkins (
    qr_token UUID PRIMARY KEY,
    event_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    scanned_at TIMESTAMPTZ,
    scanned_by_email TEXT,
    is_valid BOOLEAN NOT NULL DEFAULT true,
    
    -- Foreign keys
    CONSTRAINT fk_checkin_event
        FOREIGN KEY (event_id)
        REFERENCES events(event_id)
        ON DELETE CASCADE,
    
    CONSTRAINT fk_checkin_user
        FOREIGN KEY (user_id)
        REFERENCES users(id)
        ON DELETE CASCADE,
    
    -- One QR code per user per event
    CONSTRAINT unique_user_event_checkin
        UNIQUE (event_id, user_id)
);

-- Index for fast QR token lookups during scanning
CREATE INDEX IF NOT EXISTS idx_event_checkins_token ON event_checkins(qr_token);

-- Index for querying check-ins by event (for stats)
CREATE INDEX IF NOT EXISTS idx_event_checkins_event ON event_checkins(event_id);

-- Index for finding user's check-ins
CREATE INDEX IF NOT EXISTS idx_event_checkins_user ON event_checkins(user_id);
