-- Private recovery metadata survives account/report deletion. It has no API route.
-- Whole exports are required: sequence allocation does not imply commit order.
CREATE TABLE recovery_journal_meta (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    public_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid()
);
INSERT INTO recovery_journal_meta DEFAULT VALUES;
CREATE TABLE recovery_decisions (
    sequence BIGSERIAL PRIMARY KEY,
    public_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    action TEXT NOT NULL CHECK (action IN ('account_deleted','user_state','event_state',
        'event_deleted','report_state','report_deleted','apple_pending','apple_completed','term_state')),
    payload_ciphertext BYTEA NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

ALTER TABLE apple_revocations ADD COLUMN recovery_key TEXT;
UPDATE apple_revocations SET recovery_key = encode(sha256(
    convert_to(client_id,'UTF8') || refresh_token_ciphertext),'hex');
ALTER TABLE apple_revocations ALTER COLUMN recovery_key SET NOT NULL;
ALTER TABLE apple_revocations ADD CONSTRAINT apple_revocations_recovery_key_check
    CHECK (recovery_key ~ '^[a-f0-9]{64}$');

CREATE FUNCTION fiestaaa_recovery_record(action_name TEXT, payload JSONB)
RETURNS VOID LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO recovery_decisions(action,payload_ciphertext)
    VALUES(action_name,fiestaaa_encrypt_text(payload::text));
END $$;

CREATE FUNCTION fiestaaa_recovery_user() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        PERFORM fiestaaa_recovery_record('account_deleted',jsonb_build_object(
            'public_id',OLD.public_id,'removed_avatar',OLD.avatar_url));
        RETURN OLD;
    END IF;
    IF NEW.session_version IS DISTINCT FROM OLD.session_version
        OR NEW.suspended IS DISTINCT FROM OLD.suspended
        OR NEW.password_hash IS DISTINCT FROM OLD.password_hash
        OR NEW.password_login_enabled IS DISTINCT FROM OLD.password_login_enabled
        OR (OLD.avatar_url IS NOT NULL AND NEW.avatar_url IS DISTINCT FROM OLD.avatar_url) THEN
        PERFORM fiestaaa_recovery_record('user_state',jsonb_build_object(
            'public_id',NEW.public_id,'session_version',NEW.session_version,
            'suspended',NEW.suspended,'password_hash',NEW.password_hash,
            'password_login_enabled',NEW.password_login_enabled,'avatar_url',NEW.avatar_url,
            'removed_avatar',CASE WHEN OLD.avatar_url IS DISTINCT FROM NEW.avatar_url THEN OLD.avatar_url ELSE NULL END));
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_users AFTER UPDATE OR DELETE ON users
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_user();

CREATE FUNCTION fiestaaa_recovery_event() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        PERFORM fiestaaa_recovery_record('event_deleted',jsonb_build_object('event_id',OLD.event_id));
        RETURN OLD;
    END IF;
    IF NEW.deleted_at IS DISTINCT FROM OLD.deleted_at
        OR NEW.moderation_hidden IS DISTINCT FROM OLD.moderation_hidden THEN
        PERFORM fiestaaa_recovery_record('event_state',jsonb_build_object(
            'event_id',NEW.event_id,'moderation_hidden',NEW.moderation_hidden,
            'deleted_at',NEW.deleted_at,'deletion_reason',NEW.deletion_reason,'purge_at',NEW.purge_at));
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_events AFTER UPDATE OR DELETE ON events
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_event();

CREATE FUNCTION fiestaaa_recovery_report() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        PERFORM fiestaaa_recovery_record('report_deleted',jsonb_build_object('public_id',OLD.public_id));
        RETURN OLD;
    END IF;
    IF NEW.status IS DISTINCT FROM OLD.status OR NEW.resolved_at IS DISTINCT FROM OLD.resolved_at THEN
        PERFORM fiestaaa_recovery_record('report_state',jsonb_build_object(
            'public_id',NEW.public_id,'status',NEW.status,'resolved_at',NEW.resolved_at));
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_reports AFTER UPDATE OR DELETE ON abuse_reports
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_report();

CREATE FUNCTION fiestaaa_recovery_apple_key() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.recovery_key := encode(sha256(convert_to(NEW.client_id,'UTF8') || NEW.refresh_token_ciphertext),'hex');
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_apple_key BEFORE INSERT ON apple_revocations
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_apple_key();
CREATE FUNCTION fiestaaa_recovery_apple() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        PERFORM fiestaaa_recovery_record('apple_completed',jsonb_build_object('recovery_key',OLD.recovery_key));
        RETURN OLD;
    END IF;
    PERFORM fiestaaa_recovery_record('apple_pending',jsonb_build_object(
        'recovery_key',NEW.recovery_key,'client_id',NEW.client_id,
        'refresh_token_ciphertext',encode(NEW.refresh_token_ciphertext,'hex')));
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_apple AFTER INSERT OR DELETE ON apple_revocations
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_apple();
-- Existing pending jobs are captured with a stable key matching older snapshots.
SELECT fiestaaa_recovery_record('apple_pending',jsonb_build_object(
    'recovery_key',recovery_key,'client_id',client_id,
    'refresh_token_ciphertext',encode(refresh_token_ciphertext,'hex'))) FROM apple_revocations;

CREATE FUNCTION fiestaaa_recovery_term() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP='DELETE' THEN
        PERFORM fiestaaa_recovery_record('term_state',jsonb_build_object('term',OLD.term,'present',FALSE));
        RETURN OLD;
    END IF;
    PERFORM fiestaaa_recovery_record('term_state',jsonb_build_object('term',NEW.term,'present',TRUE));
    RETURN NEW;
END $$;
CREATE TRIGGER recovery_terms AFTER INSERT OR DELETE ON moderation_terms
    FOR EACH ROW EXECUTE FUNCTION fiestaaa_recovery_term();
