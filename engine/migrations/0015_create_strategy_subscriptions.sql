-- strategy_subscriptions: each row represents one user's opt-in to receive
-- signal-driven orders for a given strategy.  Optional per-row overrides for
-- confidence_threshold and max_position_size let each subscriber tune their own
-- risk tolerance without changing the strategy's global SafetyConfig.
CREATE TABLE IF NOT EXISTS strategy_subscriptions (
    id                   BIGSERIAL        PRIMARY KEY,
    user_id              INTEGER          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    strategy_id          TEXT             NOT NULL,
    active               BOOLEAN          NOT NULL DEFAULT true,
    max_position_size    NUMERIC,
    confidence_threshold DOUBLE PRECISION,
    created_at           TIMESTAMPTZ      NOT NULL DEFAULT now(),
    UNIQUE (user_id, strategy_id)
);

-- Speeds up the hot path: "list all active subscribers for strategy X" called
-- on every inbound signal in fan_out_signal_to_subscriptions.
CREATE INDEX IF NOT EXISTS idx_strategy_subscriptions_strategy_active
    ON strategy_subscriptions (strategy_id, active);

-- ─── Permissions ────────────────────────────────────────────────────────────
INSERT INTO permissions (name) VALUES
    ('subscriptions.write'),
    ('subscriptions.read')
ON CONFLICT (name) DO NOTHING;

-- Admin: all permissions (including the new ones).
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM   roles r, permissions p
WHERE  r.name = 'admin'
  AND  p.name IN ('subscriptions.write', 'subscriptions.read')
ON CONFLICT DO NOTHING;

-- Trader: can manage and view their own subscriptions.
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM   roles r, permissions p
WHERE  r.name = 'trader'
  AND  p.name IN ('subscriptions.write', 'subscriptions.read')
ON CONFLICT DO NOTHING;
