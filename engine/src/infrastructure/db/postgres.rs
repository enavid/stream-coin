use async_trait::async_trait;
use sqlx::PgPool;
use sqlx::Row;

use crate::infrastructure::db::ticker_repository::{
    RepositoryError, TickerRepository, TickerSubscription,
};

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
