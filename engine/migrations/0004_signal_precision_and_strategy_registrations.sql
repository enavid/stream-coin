-- Fix confidence column: REAL has only ~7 significant digits; use NUMERIC for all decimal values.
ALTER TABLE signals ALTER COLUMN confidence TYPE NUMERIC USING confidence::NUMERIC;

-- Strategy registration table: records known strategy definitions before they are started.
-- Used by POST /v1/strategies/register for both built-in and external (Python, Flink) strategies.
CREATE TABLE IF NOT EXISTS strategy_registrations (
    id            SERIAL      PRIMARY KEY,
    strategy_id   TEXT        NOT NULL UNIQUE,
    name          TEXT        NOT NULL,
    strategy_type TEXT        NOT NULL,  -- 'builtin' | 'external'
    registered_at TIMESTAMPTZ NOT NULL
);
