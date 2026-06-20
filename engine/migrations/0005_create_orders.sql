CREATE TABLE IF NOT EXISTS orders (
    id                SERIAL PRIMARY KEY,
    exchange          TEXT    NOT NULL,
    pair              TEXT    NOT NULL,
    side              TEXT    NOT NULL,
    order_type        TEXT    NOT NULL,
    quantity          NUMERIC NOT NULL,
    price             NUMERIC,
    status            TEXT    NOT NULL,
    exchange_order_id TEXT,
    client_order_id   TEXT    NOT NULL UNIQUE,
    strategy_id       TEXT,
    created_at        TIMESTAMPTZ NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS safety_config (
    id                          SERIAL PRIMARY KEY,
    max_position_size           NUMERIC NOT NULL,
    circuit_breaker_max_orders  INTEGER NOT NULL,
    circuit_breaker_window_secs INTEGER NOT NULL,
    min_confidence              REAL    NOT NULL DEFAULT 0.7,
    dry_run                     BOOLEAN NOT NULL DEFAULT true
);
