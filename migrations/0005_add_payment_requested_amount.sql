ALTER TABLE events
    ADD COLUMN IF NOT EXISTS payment_requested_amount DOUBLE PRECISION CHECK (payment_requested_amount >= 0);
