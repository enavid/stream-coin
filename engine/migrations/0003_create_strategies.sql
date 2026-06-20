CREATE TABLE IF NOT EXISTS strategies (
    id          SERIAL PRIMARY KEY,
    strategy_id TEXT        NOT NULL UNIQUE,
    strategy_type TEXT      NOT NULL,
    exchange    TEXT        NOT NULL,
    pair        TEXT        NOT NULL,
    params_json JSONB       NOT NULL,
    started_at  TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS signals (
    id          SERIAL PRIMARY KEY,
    strategy_id TEXT        NOT NULL,
    exchange    TEXT        NOT NULL,
    pair        TEXT        NOT NULL,
    action      TEXT        NOT NULL,
    confidence  REAL        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL
);
