-- Seed default categories and items for events
INSERT INTO item_types (type)
VALUES
    ('Boissons alcoolisées'),
    ('Boissons sans alcool'),
    ('Apéritifs salés'),
    ('Plats principaux'),
    ('Desserts')
ON CONFLICT (type) DO NOTHING;

INSERT INTO items (type_id, name_item, max_quantity)
SELECT type_id, name_item, max_quantity
FROM (
    VALUES
        ('Boissons alcoolisées', 'Champagne brut', 8),
        ('Boissons alcoolisées', 'Vin rouge Bordeaux', 12),
        ('Boissons alcoolisées', 'Bière artisanale IPA', 24),
        ('Boissons sans alcool', 'Jus d''orange frais', 10),
        ('Boissons sans alcool', 'Limonade maison', 12),
        ('Boissons sans alcool', 'Eau pétillante', 20),
        ('Apéritifs salés', 'Planche de fromages', 5),
        ('Apéritifs salés', 'Plateau de charcuterie', 5),
        ('Apéritifs salés', 'Chips & dips', 6),
        ('Plats principaux', 'Salade composée', 4),
        ('Plats principaux', 'Lasagnes maison', 3),
        ('Desserts', 'Tarte aux fruits', 4),
        ('Desserts', 'Brownies', 6),
        ('Desserts', 'Plateau de fruits frais', 5)
) AS seed(type_label, name_item, max_quantity)
JOIN item_types it ON it.type = seed.type_label
WHERE NOT EXISTS (
    SELECT 1
    FROM items existing
    WHERE existing.type_id = it.type_id
      AND existing.name_item = seed.name_item
);
