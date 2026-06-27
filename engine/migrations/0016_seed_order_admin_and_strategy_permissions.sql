-- Seeds two permissions that the code already enforces but migration 0008 never created:
--   orders.admin       -- used by admin order/strategy handlers (was unseeded, so the
--                         feature could never be authorized).
--   strategies.manage  -- gates strategy start/stop/register/deploy/list and backtest run.
-- Grants both to admin; trader may manage strategies and run backtests.

INSERT INTO permissions (name) VALUES
    ('orders.admin'),
    ('strategies.manage')
ON CONFLICT (name) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r, permissions p
WHERE r.name = 'admin' AND p.name IN ('orders.admin', 'strategies.manage')
ON CONFLICT DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r, permissions p
WHERE r.name = 'trader' AND p.name = 'strategies.manage'
ON CONFLICT DO NOTHING;
