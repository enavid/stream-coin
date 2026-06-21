CREATE TABLE IF NOT EXISTS python_strategies (
    id          SERIAL      PRIMARY KEY,
    strategy_id TEXT        NOT NULL UNIQUE,
    name        TEXT        NOT NULL,
    code        TEXT        NOT NULL,
    params_json JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL
);
