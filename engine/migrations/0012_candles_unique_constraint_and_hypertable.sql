-- Dedupe any rows that predate the unique constraint. The column has never
-- had a writer before this migration (CandleRepository::upsert_candles,
-- Loop 6c), so this is expected to be a no-op in practice.
DELETE FROM candles a USING candles b
WHERE a.ctid < b.ctid
  AND a.exchange = b.exchange
  AND a.pair = b.pair
  AND a.interval = b.interval
  AND a.time = b.time;

ALTER TABLE candles
    ADD CONSTRAINT candles_exchange_pair_interval_time_key
    UNIQUE (exchange, pair, interval, time);

-- Turn `candles` into a TimescaleDB hypertable now that there's a real write
-- path. The partitioning column must be `time`, already covered above.
SELECT create_hypertable('candles', 'time', if_not_exists => TRUE, migrate_data => TRUE);
