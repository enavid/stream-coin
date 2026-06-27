-- M9: persist the circuit-breaker trip so it survives a restart and is shared
-- across instances. A single-row table (id = 1) holds the latched trip bit; the
-- rolling order-count window stays in process memory.
CREATE TABLE IF NOT EXISTS circuit_breaker_state (
    id         SMALLINT    PRIMARY KEY DEFAULT 1,
    tripped    BOOLEAN     NOT NULL DEFAULT false,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT circuit_breaker_state_singleton CHECK (id = 1)
);

INSERT INTO circuit_breaker_state (id, tripped)
VALUES (1, false)
ON CONFLICT (id) DO NOTHING;
