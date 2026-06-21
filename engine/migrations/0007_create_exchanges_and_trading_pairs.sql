CREATE TABLE IF NOT EXISTS exchanges (
    id           SERIAL PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    ws_url       TEXT NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE IF NOT EXISTS trading_pairs (
    id          SERIAL PRIMARY KEY,
    exchange_id INTEGER NOT NULL REFERENCES exchanges(id),
    base        TEXT NOT NULL,
    quote       TEXT NOT NULL,
    market_type TEXT NOT NULL DEFAULT 'spot',
    active      BOOLEAN NOT NULL DEFAULT false,
    UNIQUE (exchange_id, base, quote, market_type)
);

INSERT INTO exchanges (name, display_name, ws_url, enabled)
VALUES
    ('tabdeal', 'Tabdeal', 'wss://api1.tabdeal.org/stream/', true),
    ('hitobit', 'Hitobit', 'wss://stream.hitobit.com:443', true)
ON CONFLICT (name) DO NOTHING;

INSERT INTO trading_pairs (exchange_id, base, quote, market_type, active)
SELECT id, 'USDT', 'IRT', 'spot', true FROM exchanges WHERE name = 'tabdeal'
ON CONFLICT (exchange_id, base, quote, market_type) DO NOTHING;

INSERT INTO trading_pairs (exchange_id, base, quote, market_type, active)
SELECT id, 'USDT', 'IRT', 'spot', true FROM exchanges WHERE name = 'hitobit'
ON CONFLICT (exchange_id, base, quote, market_type) DO NOTHING;
