-- Met à jour le provider Lydia sans modifier les migrations précédemment appliquées
INSERT INTO payment_providers (provider_name, url_template, validation_regex, is_active)
VALUES (
    'Lydia',
    'https://pots.lydia.me/collect/pots?id={identifier}',
    '^https?://(?:(?:collecte\\.)?lydia-app\\.com/(?:pots|collect|cagnotte|p)|pots\\.lydia\\.me/collect/pots)(?:\\?[^\\s]*)?$',
    true
)
ON CONFLICT (provider_name) DO UPDATE
SET url_template = EXCLUDED.url_template,
    validation_regex = EXCLUDED.validation_regex,
    is_active = EXCLUDED.is_active;
