ALTER TABLE invitations
    DROP CONSTRAINT IF EXISTS invitations_status_check;

ALTER TABLE invitations
    ADD CONSTRAINT invitations_status_check
    CHECK (status IN ('Waiting', 'Accepted', 'Declined', 'Expired'));
