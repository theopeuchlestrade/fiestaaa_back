-- Payment providers catalog
CREATE TABLE IF NOT EXISTS payment_providers (
    provider_id SERIAL PRIMARY KEY,
    provider_name VARCHAR(50) NOT NULL UNIQUE,
    url_template TEXT NOT NULL CHECK (url_template LIKE '%{identifier}%'),
    is_active BOOLEAN DEFAULT true
);

-- Events managed by the platform
CREATE TABLE IF NOT EXISTS events (
    event_id BIGSERIAL PRIMARY KEY,
    name_event TEXT NOT NULL,
    description TEXT NOT NULL,
    date_event DATE NOT NULL,
    start_time TIME NOT NULL,
    address TEXT NOT NULL,

    payment_provider_id INT,
    payment_identifier TEXT,

    CONSTRAINT fk_payment_provider
        FOREIGN KEY (payment_provider_id)
        REFERENCES payment_providers(provider_id)
        ON DELETE SET NULL,
    CONSTRAINT check_payment_info
        CHECK (
            (payment_provider_id IS NULL AND payment_identifier IS NULL) OR
            (payment_provider_id IS NOT NULL AND payment_identifier ~ '^[A-Za-z0-9._-]+$')
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
    type_id BIGINT NOT NULL,
    name_item TEXT NOT NULL,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    FOREIGN KEY (type_id) REFERENCES item_types(type_id) ON DELETE CASCADE
);

-- Junction table between events and items with availability constraints
CREATE TABLE IF NOT EXISTS events_items (
    event_id BIGINT NOT NULL,
    item_id BIGINT NOT NULL,
    max_quantity INT NOT NULL CHECK (max_quantity > 0),
    quantity INT DEFAULT 0,
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
