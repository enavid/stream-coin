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
use crate::infrastructure::db::order_repository::{
    OrderRecord, OrderRepository, OrderRepositoryError,
};
use crate::infrastructure::db::python_strategy_repository::{
    PythonStrategyRecord, PythonStrategyRepository, PythonStrategyRepositoryError,
};
use crate::infrastructure::db::subscription_repository::{
    SubscriptionRecord, SubscriptionRepository, SubscriptionRepositoryError,
};
use crate::infrastructure::db::ticker_repository::{
    RepositoryError, TickerRepository, TickerSubscription,
};
use crate::infrastructure::db::user_repository::{
    RoleRecord, UserRecord, UserRepository, UserRepositoryError,
};
use crate::order::circuit_breaker_store::{CircuitBreakerStore, CircuitBreakerStoreError};
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
                .map_err(|e| db_error("asset", e, AssetRepositoryError::Database))?;

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
        .map_err(|e| db_error("asset", e, AssetRepositoryError::Database))?;

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
        .map_err(|e| db_error("candle", e, CandleRepositoryError::Database))?;

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
            .map_err(|e| db_error("candle", e, CandleRepositoryError::Database))?;

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
            .map_err(|e| db_error("candle", e, CandleRepositoryError::Database))?;

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
        .map_err(|e| db_error("ticker", e, RepositoryError::Database))?;
        Ok(())
    }

    async fn remove(&self, exchange: &str, symbol: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM ticker_subscriptions WHERE exchange = $1 AND symbol = $2")
            .bind(exchange)
            .bind(symbol)
            .execute(&self.pool)
            .await
            .map_err(|e| db_error("ticker", e, RepositoryError::Database))?;
        Ok(())
    }

    async fn list_active(&self) -> Result<Vec<TickerSubscription>, RepositoryError> {
        let rows =
            sqlx::query("SELECT exchange, symbol FROM ticker_subscriptions ORDER BY started_at")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| db_error("ticker", e, RepositoryError::Database))?;

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
                .map_err(|e| db_error("exchange", e, ExchangeRepositoryError::Database))?;

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
        .map_err(|e| db_error("exchange", e, ExchangeRepositoryError::Database))?;

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
            .map_err(|e| db_error("exchange", e, ExchangeRepositoryError::Database))?;
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
        .map_err(|e| db_error("exchange", e, ExchangeRepositoryError::Database))?;
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
            .map_err(|e| db_error("exchange", e, ExchangeRepositoryError::Database))?
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
    is_unique_violation_code(e.as_database_error().and_then(|db| db.code()).as_deref())
}

/// Pure SQLSTATE check, split out so unique-violation classification is unit
/// testable without a live database (M16). `23505` is Postgres's
/// `unique_violation` class — matching on it is robust to error-message wording,
/// unlike substring checks for "unique"/"duplicate".
fn is_unique_violation_code(code: Option<&str>) -> bool {
    code == Some("23505")
}

/// Logs a database error with structured context and wraps it into the repo's
/// error type (M14). Centralizes the "log once, at the repo layer" policy so a
/// failure in a background task (order manager, restore loops) is never silently
/// dropped — previously only `upsert_candles` logged. `op` is a short
/// `"repo.method"` label for the structured log.
fn db_error<E>(op: &str, e: sqlx::Error, wrap: impl FnOnce(String) -> E) -> E {
    tracing::error!(op, error = %e, "database operation failed");
    wrap(e.to_string())
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
                db_error("user.create_user", e, UserRepositoryError::Database)
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
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

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
                .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

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
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?
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
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id)
             SELECT $1, id FROM roles WHERE name = ANY($2)",
        )
        .bind(user_id)
        .bind(role_names)
        .execute(&mut *tx)
        .await
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        tx.commit()
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;
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
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

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
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

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
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

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
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        let role_id: i32 = sqlx::query("INSERT INTO roles (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?
            .get("id");

        sqlx::query(
            "INSERT INTO role_permissions (role_id, permission_id)
             SELECT $1, id FROM permissions WHERE name = ANY($2)",
        )
        .bind(role_id)
        .bind(permissions)
        .execute(&mut *tx)
        .await
        .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        tx.commit()
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;
        Ok(())
    }

    async fn list_permissions(&self) -> Result<Vec<String>, UserRepositoryError> {
        let rows = sqlx::query("SELECT name FROM permissions ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;

        Ok(rows.into_iter().map(|r| r.get("name")).collect())
    }

    async fn user_count(&self) -> Result<i64, UserRepositoryError> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| db_error("user", e, UserRepositoryError::Database))?;
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
            .map_err(|e| db_error("credential", e, CredentialRepositoryError::Database))?
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
        .map_err(|e| db_error("credential", e, CredentialRepositoryError::Database))?;
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
        .map_err(|e| db_error("credential", e, CredentialRepositoryError::Database))?;

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
        .map_err(|e| db_error("credential", e, CredentialRepositoryError::Database))?;

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
        .map_err(|e| db_error("credential", e, CredentialRepositoryError::Database))?;
        Ok(())
    }
}

/// Postgres-backed `SubscriptionRepository`.
/// Reads and writes the `strategy_subscriptions` table (migration `0015`).
pub struct PostgresSubscriptionRepository {
    pool: PgPool,
}

impl PostgresSubscriptionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Parses an `Option<String>` (read from a NUMERIC column via `::text` cast) into
/// `Option<Decimal>`.
///
/// A SQL `NULL` (`None`) means "no value" and yields `Ok(None)`. A **non-null but
/// unparseable** string is an error (M6): the previous behaviour silently mapped
/// corruption to `None`, and for a column like `max_position_size` that `None`
/// reads as "no position cap" — silently removing a risk limit. We surface the
/// corruption instead so the caller fails closed rather than trading uncapped.
fn parse_decimal(s: Option<String>) -> Result<Option<rust_decimal::Decimal>, String> {
    match s {
        None => Ok(None),
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|e| format!("unparseable decimal {v:?}: {e}")),
    }
}

/// Builds a [`SubscriptionRecord`] from a row, failing closed if a NUMERIC
/// override (notably `max_position_size`, a risk limit) is corrupt rather than
/// silently dropping it (M6).
fn subscription_from_row(
    r: &sqlx::postgres::PgRow,
) -> Result<SubscriptionRecord, SubscriptionRepositoryError> {
    let max_position_size = parse_decimal(r.get("max_position_size")).map_err(|e| {
        SubscriptionRepositoryError::Database(format!("corrupt max_position_size: {e}"))
    })?;
    Ok(SubscriptionRecord {
        id: r.get("id"),
        user_id: r.get("user_id"),
        strategy_id: r.get("strategy_id"),
        active: r.get("active"),
        max_position_size,
        confidence_threshold: r.get("confidence_threshold"),
        created_at: r.get("created_at"),
    })
}

#[async_trait]
impl SubscriptionRepository for PostgresSubscriptionRepository {
    async fn create(
        &self,
        user_id: i32,
        strategy_id: &str,
        max_position_size: Option<rust_decimal::Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError> {
        let mps = max_position_size.map(|d| d.to_string());
        let row = sqlx::query(
            "INSERT INTO strategy_subscriptions
                 (user_id, strategy_id, max_position_size, confidence_threshold)
             VALUES ($1, $2, $3::numeric, $4)
             RETURNING id, user_id, strategy_id, active,
                       max_position_size::text AS max_position_size,
                       confidence_threshold, created_at",
        )
        .bind(user_id)
        .bind(strategy_id)
        .bind(mps)
        .bind(confidence_threshold)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            // Detect the duplicate via SQLSTATE 23505, not a brittle substring of
            // the error message (M16): the unique index name/wording can change.
            if is_unique_violation(&e) {
                SubscriptionRepositoryError::AlreadySubscribed {
                    user_id,
                    strategy_id: strategy_id.to_string(),
                }
            } else {
                db_error(
                    "subscription.create",
                    e,
                    SubscriptionRepositoryError::Database,
                )
            }
        })?;

        subscription_from_row(&row)
    }

    async fn get(
        &self,
        id: i64,
    ) -> Result<Option<SubscriptionRecord>, SubscriptionRepositoryError> {
        let row = sqlx::query(
            "SELECT id, user_id, strategy_id, active,
                    max_position_size::text AS max_position_size,
                    confidence_threshold, created_at
             FROM   strategy_subscriptions
             WHERE  id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?;

        row.map(|r| subscription_from_row(&r)).transpose()
    }

    async fn list_for_user(
        &self,
        user_id: i32,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError> {
        let rows = sqlx::query(
            "SELECT id, user_id, strategy_id, active,
                    max_position_size::text AS max_position_size,
                    confidence_threshold, created_at
             FROM   strategy_subscriptions
             WHERE  user_id = $1
             ORDER  BY created_at ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?;

        rows.iter()
            .map(subscription_from_row)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_active_for_strategy(
        &self,
        strategy_id: &str,
    ) -> Result<Vec<SubscriptionRecord>, SubscriptionRepositoryError> {
        let rows = sqlx::query(
            "SELECT id, user_id, strategy_id, active,
                    max_position_size::text AS max_position_size,
                    confidence_threshold, created_at
             FROM   strategy_subscriptions
             WHERE  strategy_id = $1
               AND  active = true
             ORDER  BY created_at ASC",
        )
        .bind(strategy_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?;

        rows.iter()
            .map(subscription_from_row)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn update(
        &self,
        id: i64,
        active: bool,
        max_position_size: Option<rust_decimal::Decimal>,
        confidence_threshold: Option<f64>,
    ) -> Result<SubscriptionRecord, SubscriptionRepositoryError> {
        let mps = max_position_size.map(|d| d.to_string());
        let row = sqlx::query(
            "UPDATE strategy_subscriptions
             SET    active = $2, max_position_size = $3::numeric, confidence_threshold = $4
             WHERE  id = $1
             RETURNING id, user_id, strategy_id, active,
                       max_position_size::text AS max_position_size,
                       confidence_threshold, created_at",
        )
        .bind(id)
        .bind(active)
        .bind(mps)
        .bind(confidence_threshold)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?
        .ok_or(SubscriptionRepositoryError::NotFound(id))?;

        subscription_from_row(&row)
    }

    async fn delete(&self, id: i64) -> Result<(), SubscriptionRepositoryError> {
        sqlx::query("DELETE FROM strategy_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?;
        Ok(())
    }

    async fn halt_for_user(&self, user_id: i32) -> Result<u64, SubscriptionRepositoryError> {
        let result = sqlx::query(
            "UPDATE strategy_subscriptions SET active = false
             WHERE user_id = $1 AND active = true",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| db_error("subscription", e, SubscriptionRepositoryError::Database))?;
        Ok(result.rows_affected())
    }
}

/// Postgres-backed `OrderRepository` (M11). Persists orders so open-order state,
/// position limits and timeout reconciliation survive a restart instead of living
/// only in process memory. NUMERIC columns are read via a `::text` cast and parsed
/// into `Decimal` (the project-wide convention, see [`parse_decimal`]).
pub struct PostgresOrderRepository {
    pool: PgPool,
}

impl PostgresOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Column list for SELECTs, with every NUMERIC column cast to text so it can
    /// be parsed losslessly into `Decimal`.
    const SELECT_COLUMNS: &'static str = "id, user_id, exchange, pair, side, order_type, \
         quantity::text AS quantity, filled_quantity::text AS filled_quantity, \
         price::text AS price, status, exchange_order_id, client_order_id, strategy_id, \
         created_at, updated_at";

    fn row_to_record(row: &sqlx::postgres::PgRow) -> Result<OrderRecord, OrderRepositoryError> {
        let parse_required = |col: &str| -> Result<rust_decimal::Decimal, OrderRepositoryError> {
            row.get::<String, _>(col).parse().map_err(|e| {
                OrderRepositoryError::Database(format!("unparseable {col} from orders row: {e}"))
            })
        };
        Ok(OrderRecord {
            id: Some(row.get::<i64, _>("id")),
            user_id: row.get::<Option<i32>, _>("user_id"),
            exchange: row.get("exchange"),
            pair: row.get("pair"),
            side: row.get("side"),
            order_type: row.get("order_type"),
            quantity: parse_required("quantity")?,
            filled_quantity: parse_required("filled_quantity")?,
            price: parse_decimal(row.get::<Option<String>, _>("price"))
                .map_err(|e| OrderRepositoryError::Database(format!("unparseable price: {e}")))?,
            status: row.get("status"),
            exchange_order_id: row.get("exchange_order_id"),
            client_order_id: row.get("client_order_id"),
            strategy_id: row.get("strategy_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }
}

#[async_trait]
impl OrderRepository for PostgresOrderRepository {
    async fn insert(&self, record: &OrderRecord) -> Result<i64, OrderRepositoryError> {
        let row = sqlx::query(
            "INSERT INTO orders
                 (user_id, exchange, pair, side, order_type, quantity, filled_quantity,
                  price, status, exchange_order_id, client_order_id, strategy_id,
                  created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6::numeric, $7::numeric, $8::numeric, $9, $10,
                     $11, $12, $13, $14)
             RETURNING id",
        )
        .bind(record.user_id)
        .bind(&record.exchange)
        .bind(&record.pair)
        .bind(&record.side)
        .bind(&record.order_type)
        .bind(record.quantity.to_string())
        .bind(record.filled_quantity.to_string())
        .bind(record.price.map(|p| p.to_string()))
        .bind(&record.status)
        .bind(record.exchange_order_id.as_deref())
        .bind(&record.client_order_id)
        .bind(record.strategy_id.as_deref())
        .bind(record.created_at)
        .bind(record.updated_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                OrderRepositoryError::DuplicateClientOrderId(record.client_order_id.clone())
            } else {
                db_error("order.insert", e, OrderRepositoryError::Database)
            }
        })?;
        Ok(row.get::<i64, _>("id"))
    }

    async fn update_status(
        &self,
        client_order_id: &str,
        status: &str,
        exchange_order_id: Option<&str>,
        fill_price: Option<rust_decimal::Decimal>,
        filled_quantity: Option<rust_decimal::Decimal>,
    ) -> Result<(), OrderRepositoryError> {
        // No fill_price column today (parity with the in-memory repo); accept it on
        // the port so a future column needs no signature change.
        let _ = fill_price;
        let result = sqlx::query(
            "UPDATE orders
             SET status = $2,
                 exchange_order_id = COALESCE($3, exchange_order_id),
                 filled_quantity = COALESCE($4::numeric, filled_quantity),
                 updated_at = NOW()
             WHERE client_order_id = $1",
        )
        .bind(client_order_id)
        .bind(status)
        .bind(exchange_order_id)
        .bind(filled_quantity.map(|q| q.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|e| db_error("order", e, OrderRepositoryError::Database))?;

        if result.rows_affected() == 0 {
            return Err(OrderRepositoryError::NotFound(client_order_id.to_string()));
        }
        Ok(())
    }

    async fn net_position(
        &self,
        user_id: Option<i32>,
        exchange: &str,
        pair: &str,
    ) -> Result<rust_decimal::Decimal, OrderRepositoryError> {
        // The exposure rules (M7/M8) live in SQL so the database is the single
        // source of truth even with multiple engine instances:
        //  - open/filled/partially_filled count the full quantity (conservative),
        //  - cancelled/failed count only what actually executed,
        //  - the user bucket is matched with IS NOT DISTINCT FROM so the NULL
        //    (system) bucket is selected by a NULL parameter.
        let row = sqlx::query(
            "SELECT COALESCE(SUM(
                 (CASE WHEN side = 'sell' THEN -1 ELSE 1 END) *
                 (CASE
                     WHEN status IN ('open', 'filled', 'partially_filled') THEN quantity
                     WHEN status IN ('cancelled', 'failed') THEN filled_quantity
                     ELSE 0
                  END)
             ), 0)::text AS net
             FROM orders
             WHERE user_id IS NOT DISTINCT FROM $1 AND exchange = $2 AND pair = $3",
        )
        .bind(user_id)
        .bind(exchange)
        .bind(pair)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| db_error("order", e, OrderRepositoryError::Database))?;

        row.get::<String, _>("net")
            .parse()
            .map_err(|e| OrderRepositoryError::Database(format!("unparseable net position: {e}")))
    }

    async fn get_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Result<OrderRecord, OrderRepositoryError> {
        let sql = format!(
            "SELECT {} FROM orders WHERE client_order_id = $1",
            Self::SELECT_COLUMNS
        );
        let row = sqlx::query(&sql)
            .bind(client_order_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| db_error("order", e, OrderRepositoryError::Database))?
            .ok_or_else(|| OrderRepositoryError::NotFound(client_order_id.to_string()))?;
        Self::row_to_record(&row)
    }

    async fn list(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<OrderRecord>, OrderRepositoryError> {
        let sql = format!(
            "SELECT {} FROM orders
             WHERE ($1::text IS NULL OR exchange = $1) AND ($2::text IS NULL OR pair = $2)
             ORDER BY id",
            Self::SELECT_COLUMNS
        );
        let rows = sqlx::query(&sql)
            .bind(exchange)
            .bind(pair)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| db_error("order", e, OrderRepositoryError::Database))?;
        rows.iter().map(Self::row_to_record).collect()
    }
}

/// Postgres-backed `PythonStrategyRepository` (M12). Persists deployed Python
/// strategy code in the `python_strategies` table (migration `0006`) so
/// `/v1/backtest/run` and the live subprocess runners can resolve a strategy by
/// id. Without this wiring `python_strategy_repository` was `None` in
/// production, permanently 400ing every backtest request.
pub struct PostgresPythonStrategyRepository {
    pool: PgPool,
}

impl PostgresPythonStrategyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn row_to_record(
        row: &sqlx::postgres::PgRow,
    ) -> Result<PythonStrategyRecord, PythonStrategyRepositoryError> {
        Ok(PythonStrategyRecord {
            strategy_id: row.get("strategy_id"),
            name: row.get("name"),
            code: row.get("code"),
            params_json: row.get("params_json"),
            created_at: row.get("created_at"),
        })
    }
}

#[async_trait]
impl PythonStrategyRepository for PostgresPythonStrategyRepository {
    async fn save(
        &self,
        record: &PythonStrategyRecord,
    ) -> Result<(), PythonStrategyRepositoryError> {
        // Idempotent upsert keyed by strategy_id: redeploying the same id
        // replaces the code/name/params rather than violating the UNIQUE
        // constraint, mirroring the in-memory Fake's retain-then-push.
        sqlx::query(
            "INSERT INTO python_strategies (strategy_id, name, code, params_json, created_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (strategy_id) DO UPDATE
             SET name = EXCLUDED.name,
                 code = EXCLUDED.code,
                 params_json = EXCLUDED.params_json",
        )
        .bind(&record.strategy_id)
        .bind(&record.name)
        .bind(&record.code)
        .bind(&record.params_json)
        .bind(record.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            db_error(
                "python_strategy",
                e,
                PythonStrategyRepositoryError::Database,
            )
        })?;
        Ok(())
    }

    async fn get(
        &self,
        strategy_id: &str,
    ) -> Result<PythonStrategyRecord, PythonStrategyRepositoryError> {
        let row = sqlx::query(
            "SELECT strategy_id, name, code, params_json, created_at
             FROM   python_strategies
             WHERE  strategy_id = $1",
        )
        .bind(strategy_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            db_error(
                "python_strategy",
                e,
                PythonStrategyRepositoryError::Database,
            )
        })?
        .ok_or_else(|| PythonStrategyRepositoryError::NotFound(strategy_id.to_string()))?;

        Self::row_to_record(&row)
    }

    async fn list_active(
        &self,
    ) -> Result<Vec<PythonStrategyRecord>, PythonStrategyRepositoryError> {
        let rows = sqlx::query(
            "SELECT strategy_id, name, code, params_json, created_at
             FROM   python_strategies
             ORDER  BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            db_error(
                "python_strategy",
                e,
                PythonStrategyRepositoryError::Database,
            )
        })?;

        rows.iter().map(Self::row_to_record).collect()
    }

    async fn remove(&self, strategy_id: &str) -> Result<(), PythonStrategyRepositoryError> {
        sqlx::query("DELETE FROM python_strategies WHERE strategy_id = $1")
            .bind(strategy_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                db_error(
                    "python_strategy",
                    e,
                    PythonStrategyRepositoryError::Database,
                )
            })?;
        Ok(())
    }
}

/// Postgres-backed `CircuitBreakerStore` (M9). Persists the latched trip bit in
/// the single-row `circuit_breaker_state` table (migration `0019`) so a trip
/// survives a restart and is shared across instances.
pub struct PostgresCircuitBreakerStore {
    pool: PgPool,
}

impl PostgresCircuitBreakerStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CircuitBreakerStore for PostgresCircuitBreakerStore {
    async fn load_tripped(&self) -> Result<bool, CircuitBreakerStoreError> {
        // No row yet (fresh deploy) reads as "not tripped".
        let row = sqlx::query("SELECT tripped FROM circuit_breaker_state WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| db_error("circuit_breaker", e, CircuitBreakerStoreError::Database))?;
        Ok(row.map(|r| r.get::<bool, _>("tripped")).unwrap_or(false))
    }

    async fn set_tripped(&self, tripped: bool) -> Result<(), CircuitBreakerStoreError> {
        sqlx::query(
            "INSERT INTO circuit_breaker_state (id, tripped, updated_at)
             VALUES (1, $1, now())
             ON CONFLICT (id) DO UPDATE
             SET tripped = EXCLUDED.tripped, updated_at = now()",
        )
        .bind(tripped)
        .execute(&self.pool)
        .await
        .map_err(|e| db_error("circuit_breaker", e, CircuitBreakerStoreError::Database))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_unique_violation_code, parse_decimal};
    use rust_decimal::Decimal;

    #[test]
    fn unique_violation_recognizes_sqlstate_23505() {
        assert!(
            is_unique_violation_code(Some("23505")),
            "23505 is the Postgres unique_violation code"
        );
    }

    #[test]
    fn unique_violation_rejects_other_sqlstates_and_none() {
        assert!(
            !is_unique_violation_code(Some("23503")),
            "foreign_key_violation must not be treated as a duplicate"
        );
        assert!(!is_unique_violation_code(Some("42P01"))); // undefined_table
        assert!(!is_unique_violation_code(None));
    }

    #[test]
    fn parse_decimal_null_is_none() {
        assert_eq!(parse_decimal(None), Ok(None));
    }

    #[test]
    fn parse_decimal_valid_value_round_trips() {
        assert_eq!(
            parse_decimal(Some("1000.50".to_string())),
            Ok(Some(Decimal::new(100050, 2)))
        );
    }

    #[test]
    fn corrupt_max_position_size_errors_not_silently_none() {
        // The bug: a non-null but unparseable risk limit became `None`, which
        // the order manager reads as "no position cap". It must error instead so
        // the caller fails closed rather than trading uncapped.
        let result = parse_decimal(Some("not-a-number".to_string()));
        assert!(
            result.is_err(),
            "a corrupt NUMERIC must be an error, never a silent None that removes the cap"
        );
    }

    #[test]
    fn parse_decimal_empty_string_errors() {
        assert!(parse_decimal(Some(String::new())).is_err());
    }
}
