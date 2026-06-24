use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::Row;

use crate::candle::entity::{Candle, CandlePayload};
use crate::exchange::registry::{ExchangeRecord, TradingPairRecord};
use crate::infrastructure::crypto::credential_cipher::EncryptedEnvelope;
use crate::infrastructure::db::asset_repository::{
    AssetRecord, AssetRepository, AssetRepositoryError,
};
use crate::infrastructure::db::candle_repository::{CandleRepository, CandleRepositoryError};
use crate::infrastructure::db::credential_repository::{
    CredentialRepository, CredentialRepositoryError, CredentialSummary,
};
use crate::infrastructure::db::exchange_repository::{ExchangeRepository, ExchangeRepositoryError};
use crate::infrastructure::db::ticker_repository::{
    RepositoryError, TickerRepository, TickerSubscription,
};
use crate::infrastructure::db::user_repository::{
    RoleRecord, UserRecord, UserRepository, UserRepositoryError,
};
use crate::price::entity::MarketType;

/// Postgres-backed `AssetRepository`. Reads the canonical `assets` table
/// (migration `0013`) — the single source of truth for coin symbol/display
/// name/decimals that `trading_pairs.base_asset_id`/`quote_asset_id`
/// (migration `0014`) reference.
pub struct PostgresAssetRepository {
    pool: PgPool,
}

impl PostgresAssetRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AssetRepository for PostgresAssetRepository {
    async fn list_all(&self) -> Result<Vec<AssetRecord>, AssetRepositoryError> {
        let rows =
            sqlx::query("SELECT id, symbol, display_name, decimals, icon_url, active FROM assets")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| AssetRepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| AssetRecord {
                id: r.get("id"),
                symbol: r.get("symbol"),
                display_name: r.get("display_name"),
                decimals: r.get("decimals"),
                icon_url: r.get("icon_url"),
                active: r.get("active"),
            })
            .collect())
    }

    async fn find_by_symbol(
        &self,
        symbol: &str,
    ) -> Result<Option<AssetRecord>, AssetRepositoryError> {
        let row = sqlx::query(
            "SELECT id, symbol, display_name, decimals, icon_url, active FROM assets WHERE symbol = $1",
        )
        .bind(symbol)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AssetRepositoryError::Database(e.to_string()))?;

        Ok(row.map(|r| AssetRecord {
            id: r.get("id"),
            symbol: r.get("symbol"),
            display_name: r.get("display_name"),
            decimals: r.get("decimals"),
            icon_url: r.get("icon_url"),
            active: r.get("active"),
        }))
    }
}

/// TimescaleDB-backed `CandleRepository`. Reads/writes the `candles`
/// hypertable, keyed by `(exchange, pair, interval, time)` per the unique
/// constraint added in migration `0012`.
pub struct PostgresCandleRepository {
    pool: PgPool,
}

impl PostgresCandleRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CandleRepository for PostgresCandleRepository {
    async fn list_candles(
        &self,
        exchange: &str,
        pair: &str,
        interval: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CandlePayload>, CandleRepositoryError> {
        let rows = sqlx::query(
            "SELECT exchange, pair, interval, time, open, high, low, close, volume
             FROM candles
             WHERE exchange = $1 AND pair = $2 AND interval = $3 AND time >= $4 AND time <= $5
             ORDER BY time",
        )
        .bind(exchange)
        .bind(pair)
        .bind(interval)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CandleRepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| CandlePayload {
                exchange: r.get("exchange"),
                pair: r.get("pair"),
                interval: r.get("interval"),
                time: r.get("time"),
                open: r.get::<i64, _>("open") as u64,
                high: r.get::<i64, _>("high") as u64,
                low: r.get::<i64, _>("low") as u64,
                close: r.get::<i64, _>("close") as u64,
                volume: r.get::<i64, _>("volume") as u64,
            })
            .collect())
    }

    async fn upsert_candles(&self, candles: &[Candle]) -> Result<(), CandleRepositoryError> {
        if candles.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CandleRepositoryError::Database(e.to_string()))?;

        for candle in candles {
            sqlx::query(
                "INSERT INTO candles (time, exchange, pair, interval, open, high, low, close, volume)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (exchange, pair, interval, time)
                 DO UPDATE SET open = EXCLUDED.open, high = EXCLUDED.high, low = EXCLUDED.low,
                               close = EXCLUDED.close, volume = EXCLUDED.volume",
            )
            .bind(candle.time)
            .bind(&candle.exchange)
            .bind(&candle.pair)
            .bind(candle.interval.as_str())
            .bind(candle.open as i64)
            .bind(candle.high as i64)
            .bind(candle.low as i64)
            .bind(candle.close as i64)
            .bind(candle.volume as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                tracing::error!(
                    exchange = %candle.exchange,
                    pair = %candle.pair,
                    interval = candle.interval.as_str(),
                    error = %e,
                    "failed to upsert candle"
                );
                CandleRepositoryError::Database(e.to_string())
            })?;
        }

        tx.commit()
            .await
            .map_err(|e| CandleRepositoryError::Database(e.to_string()))?;

        tracing::debug!(candle_count = candles.len(), "upserted candles to db");
        Ok(())
    }
}

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
            "SELECT e.name AS exchange_name, base_asset.symbol AS base, quote_asset.symbol AS quote,
                    p.market_type, p.active
             FROM trading_pairs p
             JOIN exchanges e ON e.id = p.exchange_id
             JOIN assets base_asset ON base_asset.id = p.base_asset_id
             JOIN assets quote_asset ON quote_asset.id = p.quote_asset_id",
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

    async fn upsert_pair(&self, record: &TradingPairRecord) -> Result<(), ExchangeRepositoryError> {
        let base_asset_id = self.resolve_asset_id(&record.base).await?;
        let quote_asset_id = self.resolve_asset_id(&record.quote).await?;

        sqlx::query(
            "INSERT INTO trading_pairs (exchange_id, base_asset_id, quote_asset_id, market_type, active)
             SELECT id, $2, $3, $4, $5 FROM exchanges WHERE name = $1
             ON CONFLICT (exchange_id, base_asset_id, quote_asset_id, market_type)
             DO UPDATE SET active = EXCLUDED.active",
        )
        .bind(&record.exchange_name)
        .bind(base_asset_id)
        .bind(quote_asset_id)
        .bind(record.market_type.to_string())
        .bind(record.active)
        .execute(&self.pool)
        .await
        .map_err(|e| ExchangeRepositoryError::Database(e.to_string()))?;
        Ok(())
    }
}

impl PostgresExchangeRepository {
    /// Resolves a canonical asset symbol to its `assets.id`. Returns
    /// `UnknownAsset` rather than silently inserting a NULL/invalid FK —
    /// callers must seed the symbol into `assets` first.
    async fn resolve_asset_id(&self, symbol: &str) -> Result<i32, ExchangeRepositoryError> {
        sqlx::query_scalar::<_, i32>("SELECT id FROM assets WHERE symbol = $1")
            .bind(symbol)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| ExchangeRepositoryError::Database(e.to_string()))?
            .ok_or_else(|| ExchangeRepositoryError::UnknownAsset(symbol.to_string()))
    }
}

pub struct PostgresUserRepository {
    pool: PgPool,
}

impl PostgresUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|db| db.code())
        .map(|code| code == "23505")
        .unwrap_or(false)
}

#[async_trait]
impl UserRepository for PostgresUserRepository {
    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<UserRecord, UserRepositoryError> {
        let row = sqlx::query(
            "INSERT INTO users (username, password_hash) VALUES ($1, $2)
             RETURNING id, username, password_hash, created_at",
        )
        .bind(username)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                UserRepositoryError::DuplicateUsername(username.to_string())
            } else {
                UserRepositoryError::Database(e.to_string())
            }
        })?;

        Ok(UserRecord {
            id: row.get("id"),
            username: row.get("username"),
            password_hash: row.get("password_hash"),
            created_at: row.get("created_at"),
        })
    }

    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserRecord>, UserRepositoryError> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, created_at FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(row.map(|r| UserRecord {
            id: r.get("id"),
            username: r.get("username"),
            password_hash: r.get("password_hash"),
            created_at: r.get("created_at"),
        }))
    }

    async fn list_users(&self) -> Result<Vec<UserRecord>, UserRepositoryError> {
        let rows =
            sqlx::query("SELECT id, username, password_hash, created_at FROM users ORDER BY id")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| UserRecord {
                id: r.get("id"),
                username: r.get("username"),
                password_hash: r.get("password_hash"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn assign_roles(
        &self,
        user_id: i32,
        role_names: &[String],
    ) -> Result<(), UserRepositoryError> {
        let found: Vec<String> = sqlx::query("SELECT name FROM roles WHERE name = ANY($1)")
            .bind(role_names)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?
            .into_iter()
            .map(|r| r.get::<String, _>("name"))
            .collect();

        if let Some(missing) = role_names.iter().find(|n| !found.contains(n)) {
            return Err(UserRepositoryError::RoleNotFound(missing.clone()));
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id)
             SELECT $1, id FROM roles WHERE name = ANY($2)",
        )
        .bind(user_id)
        .bind(role_names)
        .execute(&mut *tx)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;
        Ok(())
    }

    async fn roles_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError> {
        let rows = sqlx::query(
            "SELECT r.name FROM user_roles ur
             JOIN roles r ON r.id = ur.role_id
             WHERE ur.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.get("name")).collect())
    }

    async fn permissions_for_user(&self, user_id: i32) -> Result<Vec<String>, UserRepositoryError> {
        let rows = sqlx::query(
            "SELECT DISTINCT p.name FROM user_roles ur
             JOIN role_permissions rp ON rp.role_id = ur.role_id
             JOIN permissions p ON p.id = rp.permission_id
             WHERE ur.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.get("name")).collect())
    }

    async fn list_roles(&self) -> Result<Vec<RoleRecord>, UserRepositoryError> {
        let rows = sqlx::query(
            "SELECT r.name,
                    COALESCE(array_agg(p.name) FILTER (WHERE p.name IS NOT NULL), ARRAY[]::text[]) AS permissions
             FROM roles r
             LEFT JOIN role_permissions rp ON rp.role_id = r.id
             LEFT JOIN permissions p ON p.id = rp.permission_id
             GROUP BY r.name
             ORDER BY r.name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| RoleRecord {
                name: r.get("name"),
                permissions: r.get("permissions"),
            })
            .collect())
    }

    async fn create_role(
        &self,
        name: &str,
        permissions: &[String],
    ) -> Result<(), UserRepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        let role_id: i32 = sqlx::query("INSERT INTO roles (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?
            .get("id");

        sqlx::query(
            "INSERT INTO role_permissions (role_id, permission_id)
             SELECT $1, id FROM permissions WHERE name = ANY($2)",
        )
        .bind(role_id)
        .bind(permissions)
        .execute(&mut *tx)
        .await
        .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_permissions(&self) -> Result<Vec<String>, UserRepositoryError> {
        let rows = sqlx::query("SELECT name FROM permissions ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.get("name")).collect())
    }

    async fn user_count(&self) -> Result<i64, UserRepositoryError> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| UserRepositoryError::Database(e.to_string()))?;
        Ok(row.get("count"))
    }
}

pub struct PostgresCredentialRepository {
    pool: PgPool,
}

impl PostgresCredentialRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn exchange_id(&self, exchange_name: &str) -> Result<i32, CredentialRepositoryError> {
        sqlx::query("SELECT id FROM exchanges WHERE name = $1")
            .bind(exchange_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| CredentialRepositoryError::Database(e.to_string()))?
            .map(|r| r.get("id"))
            .ok_or_else(|| CredentialRepositoryError::ExchangeNotFound(exchange_name.to_string()))
    }
}

#[async_trait]
impl CredentialRepository for PostgresCredentialRepository {
    async fn upsert(
        &self,
        user_id: i32,
        exchange_name: &str,
        envelope: EncryptedEnvelope,
    ) -> Result<(), CredentialRepositoryError> {
        let exchange_id = self.exchange_id(exchange_name).await?;
        sqlx::query(
            "INSERT INTO user_exchange_credentials (user_id, exchange_id, credentials_enc, updated_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (user_id, exchange_id) DO UPDATE SET credentials_enc = $3, updated_at = NOW()",
        )
        .bind(user_id)
        .bind(exchange_id)
        .bind(sqlx::types::Json(&envelope))
        .execute(&self.pool)
        .await
        .map_err(|e| CredentialRepositoryError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<Option<EncryptedEnvelope>, CredentialRepositoryError> {
        let row = sqlx::query(
            "SELECT c.credentials_enc FROM user_exchange_credentials c
             JOIN exchanges e ON e.id = c.exchange_id
             WHERE c.user_id = $1 AND e.name = $2",
        )
        .bind(user_id)
        .bind(exchange_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CredentialRepositoryError::Database(e.to_string()))?;

        Ok(row.map(|r| {
            r.get::<sqlx::types::Json<EncryptedEnvelope>, _>("credentials_enc")
                .0
        }))
    }

    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<CredentialSummary>, CredentialRepositoryError> {
        let rows = sqlx::query(
            "SELECT e.name AS exchange_name, c.created_at FROM user_exchange_credentials c
             JOIN exchanges e ON e.id = c.exchange_id
             WHERE c.user_id = $1
             ORDER BY e.name",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CredentialRepositoryError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| CredentialSummary {
                exchange_name: r.get("exchange_name"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn delete(
        &self,
        user_id: i32,
        exchange_name: &str,
    ) -> Result<(), CredentialRepositoryError> {
        sqlx::query(
            "DELETE FROM user_exchange_credentials c
             USING exchanges e
             WHERE c.exchange_id = e.id AND c.user_id = $1 AND e.name = $2",
        )
        .bind(user_id)
        .bind(exchange_name)
        .execute(&self.pool)
        .await
        .map_err(|e| CredentialRepositoryError::Database(e.to_string()))?;
        Ok(())
    }
}
