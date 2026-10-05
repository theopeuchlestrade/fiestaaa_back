-- Additive migration: legacy sessions have version zero until reset/suspension.
ALTER TABLE users ADD COLUMN session_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN suspended BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN password_login_enabled BOOLEAN NOT NULL DEFAULT TRUE;
-- Old OAuth-created accounts stored an indistinguishable random password hash.
-- A successful password login re-enables recovery for previously linked accounts.
UPDATE users SET password_login_enabled = FALSE WHERE id IN (SELECT user_id FROM oauth_identities);
CREATE TABLE password_resets (
 user_id BIGINT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 token_hash TEXT NOT NULL UNIQUE,
 expires_at TIMESTAMPTZ NOT NULL,
 requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE apple_credentials (
 user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 client_id TEXT NOT NULL,
 refresh_token_ciphertext BYTEA NOT NULL,
 PRIMARY KEY(user_id, client_id)
);
CREATE TABLE apple_revocations (
 id BIGSERIAL PRIMARY KEY,
 client_id TEXT NOT NULL,
 refresh_token_ciphertext BYTEA NOT NULL,
 attempts INTEGER NOT NULL DEFAULT 0,
 next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE user_blocks (
 blocker_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 blocked_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 PRIMARY KEY(blocker_id, blocked_id), CHECK(blocker_id <> blocked_id)
);
CREATE INDEX user_blocks_reverse ON user_blocks(blocked_id, blocker_id);
CREATE FUNCTION fiestaaa_contact_blocked(a BIGINT, b BIGINT) RETURNS BOOLEAN
LANGUAGE SQL VOLATILE AS $$ SELECT EXISTS(SELECT 1 FROM user_blocks WHERE
(blocker_id=a AND blocked_id=b) OR (blocker_id=b AND blocked_id=a)) $$;
CREATE TABLE abuse_reports (
 public_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 reporter_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
 target_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
 event_id BIGINT REFERENCES events(event_id) ON DELETE SET NULL,
 reason TEXT NOT NULL CHECK(reason IN ('harassment','inappropriate','spam','other')),
 comment_ciphertext BYTEA NOT NULL,
 status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','resolved','dismissed')),
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
 resolved_at TIMESTAMPTZ
);
CREATE INDEX abuse_reports_open ON abuse_reports(created_at) WHERE status='open';
CREATE TABLE moderation_terms (term TEXT PRIMARY KEY CHECK(length(term)>=3));
ALTER TABLE events ADD COLUMN moderation_hidden BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE events DROP CONSTRAINT events_deletion_reason_check;
ALTER TABLE events ADD CONSTRAINT events_deletion_reason_check
 CHECK (deletion_reason IS NULL OR deletion_reason IN ('owner','retention','moderation'));
ALTER TABLE events DROP CONSTRAINT events_purge_state_check;
ALTER TABLE events ADD CONSTRAINT events_purge_state_check CHECK (
 (deleted_at IS NULL AND purge_at IS NULL AND deletion_reason IS NULL)
 OR (deleted_at IS NOT NULL AND purge_at IS NOT NULL AND deletion_reason IS NOT NULL)
 OR (moderation_hidden AND deleted_at IS NOT NULL AND purge_at IS NULL AND deletion_reason='moderation')
);

CREATE FUNCTION fiestaaa_block_cleanup() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended(LEAST(NEW.blocker_id,NEW.blocked_id)||':'||GREATEST(NEW.blocker_id,NEW.blocked_id),0));
 DELETE FROM friendships WHERE user_a=LEAST(NEW.blocker_id,NEW.blocked_id) AND user_b=GREATEST(NEW.blocker_id,NEW.blocked_id);
 DELETE FROM friend_requests WHERE (sender_id=NEW.blocker_id AND receiver_id=NEW.blocked_id) OR (sender_id=NEW.blocked_id AND receiver_id=NEW.blocker_id);
 DELETE FROM invitations i USING events e WHERE i.event_id=e.event_id AND i.status='Waiting' AND ((i.user_id=NEW.blocker_id AND e.owner_user_id=NEW.blocked_id) OR (i.user_id=NEW.blocked_id AND e.owner_user_id=NEW.blocker_id));
 DELETE FROM event_share_tokens t USING users u,events e WHERE t.event_id=e.event_id AND t.target_email_lookup_hash=u.email_lookup_hash AND ((u.id=NEW.blocker_id AND e.owner_user_id=NEW.blocked_id) OR (u.id=NEW.blocked_id AND e.owner_user_id=NEW.blocker_id));
 RETURN NEW;
END $$;
CREATE TRIGGER block_cleanup BEFORE INSERT ON user_blocks FOR EACH ROW EXECUTE FUNCTION fiestaaa_block_cleanup();
CREATE FUNCTION fiestaaa_guard_contact() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE a BIGINT; b BIGINT;
BEGIN
 IF TG_TABLE_NAME='friend_requests' THEN a:=NEW.sender_id; b:=NEW.receiver_id;
 ELSIF TG_TABLE_NAME='friendships' THEN a:=NEW.user_a; b:=NEW.user_b;
 ELSIF TG_TABLE_NAME='event_share_tokens' THEN
  IF NEW.target_email_lookup_hash IS NULL THEN RETURN NEW; END IF;
  SELECT id INTO b FROM users WHERE email_lookup_hash=NEW.target_email_lookup_hash;
  IF b IS NULL THEN RETURN NEW; END IF;
  SELECT owner_user_id INTO a FROM events WHERE event_id=NEW.event_id;
 ELSE
  IF NEW.status<>'Waiting' THEN RETURN NEW; END IF;
  SELECT owner_user_id INTO a FROM events WHERE event_id=NEW.event_id; b:=NEW.user_id;
 END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(LEAST(a,b)||':'||GREATEST(a,b),0));
 IF fiestaaa_contact_blocked(a,b) THEN RAISE EXCEPTION 'contact_unavailable' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER guard_friend_requests BEFORE INSERT OR UPDATE ON friend_requests FOR EACH ROW EXECUTE FUNCTION fiestaaa_guard_contact();
CREATE TRIGGER guard_friendships BEFORE INSERT ON friendships FOR EACH ROW EXECUTE FUNCTION fiestaaa_guard_contact();
CREATE TRIGGER guard_targeted_links BEFORE INSERT OR UPDATE ON event_share_tokens FOR EACH ROW EXECUTE FUNCTION fiestaaa_guard_contact();
CREATE TRIGGER guard_invitations BEFORE INSERT OR UPDATE ON invitations FOR EACH ROW EXECUTE FUNCTION fiestaaa_guard_contact();
CREATE FUNCTION fiestaaa_filter_text() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE content TEXT; old_content TEXT;
BEGIN
 IF TG_OP='UPDATE' AND TG_TABLE_NAME='events' THEN
  IF OLD.moderation_hidden AND NEW.deleted_at IS NULL THEN RAISE EXCEPTION 'content_hidden' USING ERRCODE='23514'; END IF;
 END IF;
 SELECT string_agg(value,' ' ORDER BY key) INTO content FROM jsonb_each_text(to_jsonb(NEW))
 WHERE key IN ('handle','name_event','description','name_item','question','label','note');
 IF TG_OP='UPDATE' THEN
  SELECT string_agg(value,' ' ORDER BY key) INTO old_content FROM jsonb_each_text(to_jsonb(OLD)) WHERE key IN ('handle','name_event','description','name_item','question','label','note');
  IF content IS NOT DISTINCT FROM old_content THEN RETURN NEW; END IF;
 END IF;
 IF EXISTS(SELECT 1 FROM moderation_terms WHERE position(term IN lower(content))>0) THEN
  RAISE EXCEPTION 'content_not_allowed' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER filter_events BEFORE INSERT OR UPDATE ON events FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
CREATE TRIGGER filter_users BEFORE INSERT OR UPDATE OF handle ON users FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
CREATE TRIGGER filter_items BEFORE INSERT OR UPDATE ON items FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
CREATE TRIGGER filter_polls BEFORE INSERT OR UPDATE ON event_polls FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
CREATE TRIGGER filter_poll_options BEFORE INSERT OR UPDATE ON event_poll_options FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
CREATE TRIGGER filter_expenses BEFORE INSERT OR UPDATE ON event_expenses FOR EACH ROW EXECUTE FUNCTION fiestaaa_filter_text();
