CREATE TABLE IF NOT EXISTS ticker_subscriptions (
    id         BIGSERIAL    PRIMARY KEY,
    exchange   TEXT         NOT NULL,
    symbol     TEXT         NOT NULL,
    started_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (exchange, symbol)
);
