ALTER TABLE items ADD COLUMN IF NOT EXISTS item_category TEXT;
UPDATE items
SET item_category = 'autre'
WHERE item_category IS NULL OR btrim(item_category) = '';

ALTER TABLE items ALTER COLUMN item_category SET DEFAULT 'autre';
ALTER TABLE items ALTER COLUMN item_category SET NOT NULL;

ALTER TABLE items DROP CONSTRAINT IF EXISTS items_item_category_check;
ALTER TABLE items
    ADD CONSTRAINT items_item_category_check
    CHECK (item_category IN ('soft', 'alcool', 'sale', 'sucre', 'autre'));
