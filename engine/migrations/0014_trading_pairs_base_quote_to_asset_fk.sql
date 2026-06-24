ALTER TABLE trading_pairs
    ADD COLUMN base_asset_id  INTEGER REFERENCES assets(id),
    ADD COLUMN quote_asset_id INTEGER REFERENCES assets(id);

UPDATE trading_pairs tp SET base_asset_id = a.id
    FROM assets a WHERE a.symbol = tp.base;
UPDATE trading_pairs tp SET quote_asset_id = a.id
    FROM assets a WHERE a.symbol = tp.quote;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM trading_pairs WHERE base_asset_id IS NULL OR quote_asset_id IS NULL) THEN
        RAISE EXCEPTION 'trading_pairs has base/quote symbols missing from assets — seed assets first';
    END IF;
END $$;

ALTER TABLE trading_pairs
    ALTER COLUMN base_asset_id  SET NOT NULL,
    ALTER COLUMN quote_asset_id SET NOT NULL;

ALTER TABLE trading_pairs
    DROP CONSTRAINT trading_pairs_exchange_id_base_quote_market_type_key,
    ADD CONSTRAINT trading_pairs_exchange_base_quote_market_key
        UNIQUE (exchange_id, base_asset_id, quote_asset_id, market_type);

ALTER TABLE trading_pairs DROP COLUMN base, DROP COLUMN quote;
