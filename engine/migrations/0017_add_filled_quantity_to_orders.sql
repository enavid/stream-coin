-- M7: track the exact cumulative executed quantity per order.
-- Needed so a partially-filled order that is later cancelled/failed keeps its
-- real residual inventory in the position accounting instead of dropping to zero.
ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS filled_quantity NUMERIC NOT NULL DEFAULT 0;
