ALTER TABLE invitations
    ADD COLUMN IF NOT EXISTS invitation_id BIGSERIAL;

CREATE UNIQUE INDEX IF NOT EXISTS invitations_invitation_id_unique
    ON invitations(invitation_id);
