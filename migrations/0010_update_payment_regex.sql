-- Met à jour tous les providers de paiement avec les nouvelles regex
-- Leetchi
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Leetchi',
    'https://www.leetchi.com/c/{identifier}',
    '^https?:\/\/(?:www\.)?leetchi\.com\/(?:c|fr\/c)\/[A-Za-z0-9_-]+\/?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;

-- Lyf
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Lyf Pay',
    'https://app.lyf.eu/p/{identifier}',
    '^https?:\/\/(?:app\.)?lyf\.eu\/(?:p|collecte)\/[A-Za-z0-9_-]+\/?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;

-- LePotCommun
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Le Pot Commun',
    'https://www.lepotcommun.fr/pot/{identifier}',
    '^https?:\/\/(?:www\.)?lepotcommun\.fr\/pot\/[A-Za-z0-9]+\/?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;

-- Papayoux
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Papayoux',
    'https://www.papayoux.com/collecte/{identifier}',
    '^https?:\/\/(?:www\.)?papayoux\.com\/(?:fr\/)?collecte\/[A-Za-z0-9_-]+\/?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;

-- Lydia (mise à jour avec la nouvelle regex)
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Lydia',
    'https://pots.lydia.me/collect/pots?id={identifier}',
    '^https?:\/\/(?:(?:collecte\.)?lydia-app\.com\/(?:pots|collect|cagnotte|p)|pots\.lydia\.me\/collect\/pots)(?:\?[^\s]*)?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;
