ALTER TABLE items ADD COLUMN IF NOT EXISTS item_kind TEXT;
UPDATE items SET item_kind = 'need' WHERE item_kind IS NULL;
ALTER TABLE items ALTER COLUMN item_kind SET DEFAULT 'need';
ALTER TABLE items ALTER COLUMN item_kind SET NOT NULL;

ALTER TABLE items
    ADD CONSTRAINT items_item_kind_check
    CHECK (item_kind IN ('need', 'bring'));
