-- Migration 0009 fixed the missing "/stream" path but kept the explicit ":443" port.
-- tokio-tungstenite sends an explicit port in the Host header whenever the URL pins
-- one, even if it's the scheme default (443 for wss) — and Hitobit's WAF returns 503
-- on that exact Host header. Dropping the explicit port fixes the reconnect loop.
UPDATE exchanges
SET ws_url = 'wss://stream.hitobit.com/stream'
WHERE name = 'hitobit' AND ws_url = 'wss://stream.hitobit.com:443/stream';
