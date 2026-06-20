CREATE TABLE IF NOT EXISTS candles (
    time        TIMESTAMPTZ NOT NULL,
    exchange    TEXT        NOT NULL,
    pair        TEXT        NOT NULL,
    interval    TEXT        NOT NULL,
    open        BIGINT      NOT NULL,
    high        BIGINT      NOT NULL,
    low         BIGINT      NOT NULL,
    close       BIGINT      NOT NULL,
    volume      BIGINT      NOT NULL
);
