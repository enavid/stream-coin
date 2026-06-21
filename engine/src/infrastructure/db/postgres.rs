use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::Row;

use crate::exchange::registry::{ExchangeRecord, TradingPairRecord};
use crate::infrastructure::db::exchange_repository::{ExchangeRepository, ExchangeRepositoryError};
use crate::infrastructure::db::ticker_repository::{
    RepositoryError, TickerRepository, TickerSubscription,
};
use crate::price::entity::MarketType;

pub struct PostgresTickerRepository {
    pool: PgPool,
}

impl PostgresTickerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TickerRepository for PostgresTickerRepository {
    async fn insert(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO ticker_subscriptions (exchange, symbol) VALUES ($1, $2)
             ON CONFLICT (exchange, symbol) DO NOTHING",
        )
        .bind(exchange)
        .bind(symbol)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(())
    }

    async fn remove(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM ticker_subscriptions WHERE exchange = $1 AND symbol = $2")
            .bind(exchange)
            .bind(symbol)
            .execute(&self.pool)
            .await
            .map_err(|e| RepositoryError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<TickerSubscription>, RepositoryError> {
        let rows =
            sqlx::query("SELECT exchange, symbol FROM ticker_subscriptions ORDER BY started_at")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| RepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| TickerSubscription {
                exchange: r.get("exchange"),
                symbol: r.get("symbol"),
            })
            .collect())
    }
}

fn parse_market_type(s: &str) -> MarketType {
    match s {
        "futures" => MarketType::Futures,
        "swap" => MarketType::Swap,
        _ => MarketType::Spot,
    }
}

pub struct PostgresExchangeRepository {
    pool: PgPool,
}

impl PostgresExchangeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ExchangeRepository for PostgresExchangeRepository {
    async fn load_all(
        &self,
    ) -> Result<(Vec<ExchangeRecord>, Vec<TradingPairRecord>), ExchangeRepositoryError> {
        let exchange_rows =
            sqlx::query("SELECT name, display_name, ws_url, enabled FROM exchanges")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| ExchangeRepositoryError::Database(e.to_string()))?;

        let exchanges: Vec<ExchangeRecord> = exchange_rows
            .into_iter()
            .map(|r| ExchangeRecord {
                name: r.get("name"),
                display_name: r.get("display_name"),
                ws_url: r.get("ws_url"),
                enabled: r.get("enabled"),
            })
            .collect();

        let pair_rows = sqlx::query(
            "SELECT e.name AS exchange_name, p.base, p.quote, p.market_type, p.active
             FROM trading_pairs p
             JOIN exchanges e ON e.id = p.exchange_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ExchangeRepositoryError::Database(e.to_string()))?;

        let pairs: Vec<TradingPairRecord> = pair_rows
            .into_iter()
            .map(|r| TradingPairRecord {
                exchange_name: r.get("exchange_name"),
                base: r.get("base"),
                quote: r.get("quote"),
                market_type: parse_market_type(r.get("market_type")),
                active: r.get("active"),
            })
            .collect();

        Ok((exchanges, pairs))
    }

    async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), ExchangeRepositoryError> {
        sqlx::query("UPDATE exchanges SET enabled = $1 WHERE name = $2")
            .bind(enabled)
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| ExchangeRepositoryError::Database(e.to_string()))?;
        Ok(())
    }
}
