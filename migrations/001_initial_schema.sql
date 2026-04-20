-- Final clean-slate schema for Fiestaaa Back.
-- This migration is the single source of truth after the global reset.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION fiestaaa_normalize_email(value TEXT)
RETURNS TEXT
LANGUAGE SQL
IMMUTABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT lower(trim(value));
$$;

CREATE OR REPLACE FUNCTION fiestaaa_encrypt_text(value TEXT)
RETURNS BYTEA
LANGUAGE SQL
VOLATILE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT pgp_sym_encrypt(
        value,
        current_setting('fiestaaa.data_encryption_key'),
        'cipher-algo=aes256, compress-algo=0'
    );
$$;

CREATE OR REPLACE FUNCTION fiestaaa_decrypt_text(value BYTEA)
RETURNS TEXT
LANGUAGE SQL
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT pgp_sym_decrypt(
        value,
        current_setting('fiestaaa.data_encryption_key')
    );
$$;

CREATE OR REPLACE FUNCTION fiestaaa_lookup_text(value TEXT)
RETURNS TEXT
LANGUAGE SQL
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT encode(
        hmac(value, current_setting('fiestaaa.data_lookup_key'), 'sha256'),
        'hex'
    );
$$;

CREATE OR REPLACE FUNCTION fiestaaa_email_lookup(value TEXT)
RETURNS TEXT
LANGUAGE SQL
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT fiestaaa_lookup_text(fiestaaa_normalize_email(value));
$$;

CREATE OR REPLACE FUNCTION fiestaaa_email_matches(stored_hash TEXT, candidate TEXT)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT stored_hash = fiestaaa_email_lookup(candidate);
$$;

CREATE OR REPLACE FUNCTION fiestaaa_lookup_matches(stored_hash TEXT, candidate TEXT)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
RETURNS NULL ON NULL INPUT
AS $$
    SELECT stored_hash = fiestaaa_lookup_text(candidate);
$$;

CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    public_id UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    email_ciphertext BYTEA NOT NULL,
    email_lookup_hash TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    handle TEXT NOT NULL,
    avatar_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX users_handle_unique_idx ON users (lower(handle));

CREATE TABLE payment_providers (
    provider_id SERIAL PRIMARY KEY,
    provider_name VARCHAR(50) NOT NULL UNIQUE,
    url_template TEXT NOT NULL CHECK (url_template LIKE '%{identifier}%'),
    validation_regex TEXT NOT NULL DEFAULT '^https?://.+$',
    is_active BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE TABLE events (
    event_id BIGSERIAL PRIMARY KEY,
    name_event TEXT NOT NULL,
    description TEXT NOT NULL,
    date_event DATE NOT NULL,
    start_time TIME NOT NULL,
    end_date DATE,
    end_time TIME,
    invitation_deadline DATE,
    address_ciphertext BYTEA NOT NULL,
    owner_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    payment_provider_id INT REFERENCES payment_providers(provider_id) ON DELETE SET NULL,
    payment_identifier_ciphertext BYTEA,
    payment_requested_amount DOUBLE PRECISION CHECK (payment_requested_amount >= 0),
    payment_per_person BOOLEAN NOT NULL DEFAULT FALSE,
    latitude_ciphertext BYTEA,
    longitude_ciphertext BYTEA,
    playlist_url TEXT,
    playlist_provider TEXT,
    enabled_features TEXT[] NOT NULL DEFAULT ARRAY['carpools', 'polls', 'items']::TEXT[],
    CONSTRAINT check_payment_info
        CHECK (payment_provider_id IS NOT NULL OR payment_identifier_ciphertext IS NULL),
    CONSTRAINT events_playlist_provider_check
        CHECK (
            (playlist_url IS NULL AND playlist_provider IS NULL)
            OR (
                playlist_url IS NOT NULL
                AND length(trim(playlist_url)) > 0
                AND playlist_provider IN ('spotify', 'apple_music', 'deezer')
            )
        ),
    CONSTRAINT events_enabled_features_check
        CHECK (
            enabled_features <@ ARRAY[
                'carpools',
                'polls',
                'items',
                'ticketing',
                'playlist',
                'payment',
                'expenses'
            ]::TEXT[]
        ),
    CONSTRAINT events_end_datetime_check
        CHECK (
            (end_date IS NULL AND end_time IS NULL)
            OR (
                end_date IS NOT NULL
                AND end_time IS NOT NULL
                AND (end_date, end_time) >= (date_event, start_time)
            )
        ),
    CONSTRAINT invitation_deadline_before_event
        CHECK (invitation_deadline IS NULL OR invitation_deadline <= date_event)
);

CREATE INDEX idx_events_owner_user_id ON events(owner_user_id);

CREATE TABLE item_types (
    type_id BIGSERIAL PRIMARY KEY,
    type TEXT UNIQUE NOT NULL
);

CREATE TABLE items (
    item_id BIGSERIAL PRIMARY KEY,
    type_id BIGINT NOT NULL REFERENCES item_types(type_id) ON DELETE CASCADE,
    name_item TEXT NOT NULL,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    unit_label TEXT NOT NULL DEFAULT 'unités',
    item_kind TEXT NOT NULL DEFAULT 'need',
    CONSTRAINT items_item_kind_check
        CHECK (item_kind IN ('need', 'bring'))
);

CREATE TABLE events_items (
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    item_id BIGINT NOT NULL REFERENCES items(item_id) ON DELETE CASCADE,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    quantity INT DEFAULT 0,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (event_id, item_id)
);

CREATE TABLE user_items (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_id BIGINT NOT NULL,
    item_id BIGINT NOT NULL,
    quantity INT NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (user_id, event_id, item_id),
    FOREIGN KEY (event_id, item_id) REFERENCES events_items(event_id, item_id) ON DELETE CASCADE
);

CREATE TABLE invitations (
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status VARCHAR(50) DEFAULT 'Waiting',
    date_invi TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (event_id, user_id),
    CONSTRAINT invitations_status_check
        CHECK (status IN ('Waiting', 'Accepted', 'Declined', 'Expired'))
);

CREATE TABLE event_share_tokens (
    token_hash TEXT PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    created_by_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    used_at TIMESTAMPTZ,
    used_by_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    target_email_ciphertext BYTEA,
    target_email_lookup_hash TEXT
);

CREATE INDEX idx_event_share_tokens_event ON event_share_tokens(event_id);
CREATE INDEX idx_event_share_tokens_target_email_lookup_hash
    ON event_share_tokens(target_email_lookup_hash);

CREATE TABLE event_checkins (
    qr_token_hash TEXT PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    scanned_at TIMESTAMPTZ,
    scanned_by_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    is_valid BOOLEAN NOT NULL DEFAULT TRUE,
    CONSTRAINT unique_user_event_checkin UNIQUE (event_id, user_id)
);

CREATE INDEX idx_event_checkins_event ON event_checkins(event_id);
CREATE INDEX idx_event_checkins_user ON event_checkins(user_id);

CREATE TABLE user_devices (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    fcm_token_ciphertext BYTEA NOT NULL,
    fcm_token_lookup_hash TEXT NOT NULL UNIQUE,
    platform TEXT NOT NULL CHECK (platform IN ('ios', 'android', 'web')),
    locale TEXT,
    app_version TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    disabled_at TIMESTAMPTZ
);

CREATE INDEX idx_user_devices_user ON user_devices(user_id);
CREATE INDEX idx_user_devices_active ON user_devices(user_id) WHERE disabled_at IS NULL;

CREATE TABLE event_polls (
    poll_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    allow_multiple BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_event_polls_event ON event_polls(event_id);
CREATE INDEX idx_event_polls_expires_at ON event_polls(expires_at);

CREATE TABLE event_poll_options (
    option_id BIGSERIAL PRIMARY KEY,
    poll_id BIGINT NOT NULL REFERENCES event_polls(poll_id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    position INT NOT NULL DEFAULT 0
);

CREATE INDEX idx_event_poll_options_poll ON event_poll_options(poll_id);

CREATE TABLE event_poll_votes (
    poll_id BIGINT NOT NULL REFERENCES event_polls(poll_id) ON DELETE CASCADE,
    option_id BIGINT NOT NULL REFERENCES event_poll_options(option_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (poll_id, user_id, option_id)
);

CREATE INDEX idx_event_poll_votes_poll ON event_poll_votes(poll_id);
CREATE INDEX idx_event_poll_votes_user ON event_poll_votes(user_id);

CREATE TABLE carpools (
    carpool_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    driver_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    origin_ciphertext BYTEA NOT NULL,
    origin_latitude_ciphertext BYTEA,
    origin_longitude_ciphertext BYTEA,
    depart_at TIMESTAMPTZ NOT NULL,
    seats_total INT NOT NULL CHECK (seats_total > 0),
    seats_taken INT NOT NULL DEFAULT 0 CHECK (seats_taken >= 0 AND seats_taken <= seats_total),
    notes_ciphertext BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_carpools_event ON carpools(event_id);
CREATE INDEX idx_carpools_driver ON carpools(driver_id);

CREATE TABLE carpool_passengers (
    carpool_id BIGINT NOT NULL REFERENCES carpools(carpool_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (carpool_id, user_id)
);

CREATE INDEX idx_carpool_passengers_user ON carpool_passengers(user_id);

CREATE TABLE event_expenses (
    expense_id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    paid_by_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_by_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    amount_cents BIGINT NOT NULL CHECK (amount_cents > 0),
    note TEXT,
    expense_date TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_event_expenses_event ON event_expenses(event_id, expense_date DESC);
CREATE INDEX idx_event_expenses_creator ON event_expenses(created_by_user_id);

CREATE TABLE event_expense_participants (
    expense_id BIGINT NOT NULL REFERENCES event_expenses(expense_id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    share_weight INT NOT NULL DEFAULT 1 CHECK (share_weight > 0),
    PRIMARY KEY (expense_id, user_id)
);

CREATE INDEX idx_event_expense_participants_user ON event_expense_participants(user_id);

CREATE TABLE oauth_identities (
    provider TEXT NOT NULL,
    provider_subject_ciphertext BYTEA NOT NULL,
    provider_subject_lookup_hash TEXT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, provider_subject_lookup_hash)
);

CREATE INDEX idx_oauth_identities_user_id ON oauth_identities(user_id);

CREATE TABLE pending_registrations (
    email_lookup_hash TEXT PRIMARY KEY,
    email_ciphertext BYTEA NOT NULL,
    password_hash TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    verification_token_hash TEXT NOT NULL UNIQUE,
    verification_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE revoked_auth_tokens (
    token_hash TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX revoked_auth_tokens_expires_at_idx
    ON revoked_auth_tokens(expires_at);

CREATE TABLE friendships (
    user_a BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT friendships_user_order CHECK (user_a < user_b),
    CONSTRAINT friendships_no_self CHECK (user_a <> user_b),
    CONSTRAINT friendships_pk PRIMARY KEY (user_a, user_b)
);

CREATE INDEX friendships_user_lookup_idx
    ON friendships(user_a, user_b);

CREATE TABLE friend_requests (
    id BIGSERIAL PRIMARY KEY,
    sender_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    receiver_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'Pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    responded_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX friend_requests_pending_pair_idx
    ON friend_requests (LEAST(sender_id, receiver_id), GREATEST(sender_id, receiver_id))
    WHERE status = 'Pending';

INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES
    (
        'Lydia',
        'https://pots.lydia.me/collect/pots?id={identifier}',
        '^https?:\/\/(?:(?:collecte\.)?lydia-app\.com\/(?:pots|collect|cagnotte|p)|pots\.lydia\.me\/collect\/pots)(?:\?[^\s]*)?$',
        TRUE
    ),
    (
        'Leetchi',
        'https://www.leetchi.com/c/{identifier}',
        '^https?:\/\/(?:www\.)?leetchi\.com\/(?:c|fr\/c)\/[A-Za-z0-9_-]+\/?$',
        TRUE
    ),
    (
        'Lyf Pay',
        'https://app.lyf.eu/p/{identifier}',
        '^https?:\/\/(?:app\.)?lyf\.eu\/(?:p|collecte)\/[A-Za-z0-9_-]+\/?$',
        TRUE
    ),
    (
        'Le Pot Commun',
        'https://www.lepotcommun.fr/pot/{identifier}',
        '^https?:\/\/(?:www\.)?lepotcommun\.fr\/pot\/[A-Za-z0-9]+\/?$',
        TRUE
    ),
    (
        'Papayoux',
        'https://www.papayoux.com/collecte/{identifier}',
        '^https?:\/\/(?:www\.)?papayoux\.com\/(?:fr\/)?collecte\/[A-Za-z0-9_-]+\/?$',
        TRUE
    )
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;
