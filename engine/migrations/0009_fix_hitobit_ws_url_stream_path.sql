-- Migration 0007 seeded hitobit's ws_url without the required "/stream" path.
-- The WS upgrade only succeeds under "/stream" (root rejects with 404/503),
-- which is why ticker start for hitobit kept reconnecting in a loop.
UPDATE exchanges
SET ws_url = 'wss://stream.hitobit.com:443/stream'
WHERE name = 'hitobit' AND ws_url = 'wss://stream.hitobit.com:443';
