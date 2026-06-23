INSERT INTO exchanges (name, display_name, ws_url, enabled)
VALUES ('coinex', 'CoinEx', 'wss://socket.coinex.com/v2/spot', false)
ON CONFLICT (name) DO NOTHING;
