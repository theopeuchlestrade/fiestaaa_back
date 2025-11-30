-- Reset migration consolidating the full schema and seed data

-- Users table (handle required and unique case-insensitive)
CREATE TABLE IF NOT EXISTS users (
    id BIGSERIAL PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    handle TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS users_handle_unique_idx ON users (lower(handle));

-- Payment providers catalog
CREATE TABLE IF NOT EXISTS payment_providers (
    provider_id SERIAL PRIMARY KEY,
    provider_name VARCHAR(50) NOT NULL UNIQUE,
    url_template TEXT NOT NULL CHECK (url_template LIKE '%{identifier}%'),
    validation_regex TEXT NOT NULL DEFAULT '^https?://.+$',
    is_active BOOLEAN NOT NULL DEFAULT true
);

-- Events managed by the platform
CREATE TABLE IF NOT EXISTS events (
    event_id BIGSERIAL PRIMARY KEY,
    name_event TEXT NOT NULL,
    description TEXT NOT NULL,
    date_event DATE NOT NULL,
    start_time TIME NOT NULL,
    address TEXT NOT NULL,
    owner_email TEXT NOT NULL DEFAULT '',

    payment_provider_id INT,
    payment_identifier TEXT,
    payment_requested_amount DOUBLE PRECISION CHECK (payment_requested_amount >= 0),
    payment_per_person BOOLEAN NOT NULL DEFAULT FALSE,
    latitude DOUBLE PRECISION,
    longitude DOUBLE PRECISION,

    CONSTRAINT fk_payment_provider
        FOREIGN KEY (payment_provider_id)
        REFERENCES payment_providers(provider_id)
        ON DELETE SET NULL,
    CONSTRAINT check_payment_info
        CHECK (
            (payment_identifier IS NULL AND payment_provider_id IS NULL)
            OR (payment_provider_id IS NOT NULL AND payment_identifier IS NULL)
            OR (payment_provider_id IS NOT NULL AND length(trim(payment_identifier)) > 0)
        )
);

-- Reference list for item categories
CREATE TABLE IF NOT EXISTS item_types (
    type_id BIGSERIAL PRIMARY KEY,
    type TEXT UNIQUE NOT NULL
);

-- Items that can be attached to events
CREATE TABLE IF NOT EXISTS items (
    item_id BIGSERIAL PRIMARY KEY,
    type_id BIGINT NOT NULL REFERENCES item_types(type_id) ON DELETE CASCADE,
    name_item TEXT NOT NULL,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    unit_label TEXT NOT NULL DEFAULT 'unités'
);

-- Junction table between events and items with availability constraints
CREATE TABLE IF NOT EXISTS events_items (
    event_id BIGINT NOT NULL,
    item_id BIGINT NOT NULL,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    quantity INT DEFAULT 0,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (event_id, item_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id) ON DELETE CASCADE,
    FOREIGN KEY (item_id) REFERENCES items(item_id) ON DELETE CASCADE
);

-- Items booked by users for specific events
CREATE TABLE IF NOT EXISTS user_items (
    user_id BIGINT NOT NULL,
    event_id BIGINT NOT NULL,
    item_id BIGINT NOT NULL,
    quantity INT NOT NULL CHECK (quantity > 0),
    PRIMARY KEY (user_id, event_id, item_id),
    FOREIGN KEY (event_id, item_id) REFERENCES events_items(event_id, item_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Invitations sent to users for events
CREATE TABLE IF NOT EXISTS invitations (
    event_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    status VARCHAR(50) DEFAULT 'Waiting' CHECK (status IN ('Waiting', 'Accepted', 'Declined')),
    date_invi TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (event_id, user_id),
    FOREIGN KEY (event_id) REFERENCES events(event_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Share tokens for public event invitations
CREATE TABLE IF NOT EXISTS event_share_tokens (
    token UUID PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
    created_by_email TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    used_at TIMESTAMPTZ,
    used_by_email TEXT
);

CREATE INDEX IF NOT EXISTS idx_event_share_tokens_event ON event_share_tokens(event_id);

-- Seed payment providers with final URL templates and validation regexes
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES
    (
        'Lydia',
        'https://pots.lydia.me/collect/pots?id={identifier}',
        '^https?:\/\/(?:(?:collecte\\.)?lydia-app\.com\/(?:pots|collect|cagnotte|p)|pots\.lydia\.me\/collect\/pots)(?:\?[^\s]*)?$',
        true
    ),
    (
        'Leetchi',
        'https://www.leetchi.com/c/{identifier}',
        '^https?:\/\/(?:www\.)?leetchi\.com\/(?:c|fr\/c)\/[A-Za-z0-9_-]+\/?$',
        true
    ),
    (
        'Lyf Pay',
        'https://app.lyf.eu/p/{identifier}',
        '^https?:\/\/(?:app\.)?lyf\.eu\/(?:p|collecte)\/[A-Za-z0-9_-]+\/?$',
        true
    ),
    (
        'Le Pot Commun',
        'https://www.lepotcommun.fr/pot/{identifier}',
        '^https?:\/\/(?:www\.)?lepotcommun\.fr\/pot\/[A-Za-z0-9]+\/?$',
        true
    ),
    (
        'Papayoux',
        'https://www.papayoux.com/collecte/{identifier}',
        '^https?:\/\/(?:www\.)?papayoux\.com\/(?:fr\/)?collecte\/[A-Za-z0-9_-]+\/?$',
        true
    )
ON CONFLICT (provider_name) DO UPDATE
SET url_template     = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active        = EXCLUDED.is_active;
