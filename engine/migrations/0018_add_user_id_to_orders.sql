-- M8: scope the position limit per user. A NULL user_id marks a system /
-- signal-driven order (no owning user); a non-NULL value scopes the order to
-- one user so their position limit is computed independently of everyone else.
ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS user_id INTEGER REFERENCES users(id) ON DELETE SET NULL;

-- Supports the net_position query (sum by user bucket + exchange + pair + status).
CREATE INDEX IF NOT EXISTS idx_orders_user_exchange_pair_status
    ON orders (user_id, exchange, pair, status);
