-- Event-bound carpooling for participant rides

CREATE TABLE IF NOT EXISTS carpools (
    carpool_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    driver_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    origin TEXT NOT NULL,
    origin_latitude DOUBLE PRECISION,
    origin_longitude DOUBLE PRECISION,
    depart_at TIMESTAMPTZ NOT NULL,
    seats_total INT NOT NULL CHECK (seats_total > 0),
    seats_taken INT NOT NULL DEFAULT 0 CHECK (seats_taken >= 0 AND seats_taken <= seats_total),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_carpools_event ON carpools(event_id);
CREATE INDEX IF NOT EXISTS idx_carpools_driver ON carpools(driver_id);

CREATE TABLE IF NOT EXISTS carpool_passengers (
    carpool_id BIGINT NOT NULL REFERENCES carpools(carpool_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (carpool_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_carpool_passengers_user ON carpool_passengers(user_id);
