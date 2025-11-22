ALTER TABLE payment_providers
    ADD COLUMN IF NOT EXISTS validation_regex TEXT NOT NULL DEFAULT '^https?://.+$';

ALTER TABLE events
    DROP CONSTRAINT IF EXISTS check_payment_info;

ALTER TABLE events
    ADD CONSTRAINT check_payment_info
        CHECK (
            (payment_identifier IS NULL AND payment_provider_id IS NULL)
            OR (payment_provider_id IS NOT NULL AND payment_identifier IS NULL)
            OR (payment_provider_id IS NOT NULL AND length(trim(payment_identifier)) > 0)
        );

INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES
    (
        'Lydia',
        'https://lydia-app.com/pots?id={identifier}',
        '^https?://(?:collecte\\.)?lydia-app\\.com/(?:pots|collect|cagnotte|p)[^\\s]*$',
        true
    ),
    (
        'Leetchi',
        'https://www.leetchi.com/c/{identifier}',
        '^https?://(?:www\\.)?leetchi\\.com/(?:c|fr/c)/[A-Za-z0-9_-]+/?$',
        true
    ),
    (
        'Lyf Pay',
        'https://www.lyf.eu/p/{identifier}',
        '^https?://(?:app\\.)?lyf\\.eu/(?:p|collecte)/[A-Za-z0-9_-]+/?$',
        true
    ),
    (
        'Le Pot Commun',
        'https://www.lepotcommun.fr/pot/{identifier}',
        '^https?://(?:www\\.)?lepotcommun\\.fr/pot/[A-Za-z0-9]+/?$',
        true
    ),
    (
        'Papayoux',
        'https://www.papayoux.com/fr/collecte/{identifier}',
        '^https?://(?:www\\.)?papayoux\\.com/(?:fr/)?collecte/[A-Za-z0-9_-]+/?$',
        true
    )
ON CONFLICT (provider_name) DO UPDATE
SET url_template      = EXCLUDED.url_template,
    validation_regex  = EXCLUDED.validation_regex,
    is_active         = EXCLUDED.is_active;
