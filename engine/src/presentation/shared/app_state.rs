use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::AbortHandle;

use crate::candle::entity::CandlePayload;
use crate::exchange::historical_port::HistoricalCandleSource;
use crate::exchange::market_seed_port::TopMarketSource;
use crate::exchange::port::ExchangeAdapter;
use crate::exchange::registry::ExchangeRegistry;
use crate::infrastructure::crypto::credential_cipher::CredentialCipher;
use crate::infrastructure::db::candle_repository::CandleRepository;
use crate::infrastructure::db::credential_repository::CredentialRepository;
use crate::infrastructure::db::exchange_repository::ExchangeRepository;
use crate::infrastructure::db::python_strategy_repository::PythonStrategyRepository;
use crate::infrastructure::db::signal_repository::SignalRepository;
use crate::infrastructure::db::strategy_repository::StrategyRepository;
use crate::infrastructure::db::ticker_repository::TickerRepository;
use crate::infrastructure::db::user_repository::UserRepository;
use crate::kafka::port::MessagePublisher;
use crate::order::manager::OrderManager;
use crate::order::port::OrderAdapter;

pub type ClientKey = String;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, AbortHandle>>>;

/// In-memory ring buffer of recently closed candles, keyed by
/// `"{exchange}:{pair}:{interval}"`, newest at the back. This is separate from
/// `candle_repository` (the persistent store used by the backtest engine,
/// currently unconfigured in production) — it exists so the live chart page
/// has *something* to seed from even without a database, and is capped per
/// key (see `CANDLE_HISTORY_CAPACITY`) rather than growing unboundedly.
pub type CandleHistory = Arc<Mutex<HashMap<String, VecDeque<CandlePayload>>>>;

/// Max candles retained per `(exchange, pair, interval)` key in `CandleHistory`.
pub const CANDLE_HISTORY_CAPACITY: usize = 500;

/// Factory function that constructs an `ExchangeAdapter` given a WebSocket URL.
pub type AdapterFactory = Arc<dyn Fn(&str) -> Arc<dyn ExchangeAdapter> + Send + Sync>;

/// Capacity of the price broadcast channel; lagging WS clients drop oldest
/// messages rather than blocking publishers.
pub const BROADCAST_CAPACITY: usize = 256;

/// Metadata for a running strategy, stored alongside its abort handle.
#[derive(Clone, Debug)]
pub struct StrategyHandle {
    pub strategy_type: String,
    pub exchange: String,
    pub pair: String,
    pub abort_handle: AbortHandle,
}

/// Shared server state injected into every actix-web handler via `web::Data`.
#[derive(Clone)]
pub struct AppState {
    /// Raw Redis connection used by the health check; `None` when Redis is unavailable.
    pub redis: Option<MultiplexedConnection>,
    /// Registry of live exchange adapters keyed by exchange name (e.g. `"tabdeal"`).
    /// Adapters are inserted on enable and removed on disable — use `RwLock` for
    /// cheap concurrent reads with exclusive writes only on enable/disable.
    pub exchange_adapters: Arc<RwLock<HashMap<String, Arc<dyn ExchangeAdapter>>>>,
    /// In-memory registry of known exchanges and trading pairs, with enable/disable state.
    pub exchange_registry: Arc<Mutex<ExchangeRegistry>>,
    /// Hard-coded factory map: exchange name → constructor. The registry drives
    /// which exchanges are active; this map provides the construction logic.
    pub adapter_factories: Arc<HashMap<String, AdapterFactory>>,
    /// Handles of currently-running ticker subscriptions, keyed by `"exchange:symbol"`.
    pub clients: ClientMap,
    /// Kafka publisher; `None` when `KAFKA_URL` is unset or the broker is unreachable.
    pub publisher: Option<Arc<dyn MessagePublisher>>,
    /// Broadcast channel that fans out every serialized price tick to all WS sessions.
    pub broadcaster: broadcast::Sender<String>,
    /// HS256 secret for JWT validation. `None` = auth disabled (development mode).
    pub jwt_secret: Option<Arc<String>>,
    /// Persistent store for active ticker subscriptions. `None` = in-memory only (no DB).
    pub ticker_repository: Option<Arc<dyn TickerRepository>>,
    /// Handles of currently-running strategy tasks, keyed by `strategy_id`.
    pub running_strategies: Arc<Mutex<HashMap<String, StrategyHandle>>>,
    /// Persistent store for active strategy records. `None` = in-memory only (no DB).
    pub strategy_repository: Option<Arc<dyn StrategyRepository>>,
    /// Persistent store for emitted signals. `None` = signals not persisted to DB.
    pub signal_repository: Option<Arc<dyn SignalRepository>>,
    /// Live order adapters keyed by exchange name (e.g. `"tabdeal"`).
    /// Writable at runtime via `POST /v1/admin/exchanges/{name}/credentials` so
    /// operators can register API keys without restarting the server.
    pub order_adapters: Arc<RwLock<HashMap<String, Arc<dyn OrderAdapter>>>>,
    /// Order Manager — processes signals into orders, enforces safety controls.
    /// `None` when no `OrderRepository` is available or in test stubs that do not
    /// exercise order placement.
    pub order_manager: Option<Arc<OrderManager>>,
    /// Persistent store for deployed Python strategy code. `None` = no DB.
    pub python_strategy_repository: Option<Arc<dyn PythonStrategyRepository>>,
    /// Historical candle store used by the backtest engine. `None` = backtesting
    /// unavailable (no DB configured).
    pub candle_repository: Option<Arc<dyn CandleRepository>>,
    /// Persistent store for the exchange/trading-pair registry. `None` = the in-memory
    /// `exchange_registry` is bootstrapped from hardcoded defaults and enable/disable
    /// changes do not survive a restart.
    pub exchange_repository: Option<Arc<dyn ExchangeRepository>>,
    /// Persistent store for users, roles, and permissions. `None` = no DB — login and
    /// user-management endpoints are unavailable.
    pub user_repository: Option<Arc<dyn UserRepository>>,
    /// Persistent store for per-user encrypted exchange credentials. `None` = no DB.
    pub credential_repository: Option<Arc<dyn CredentialRepository>>,
    /// AES-256-GCM cipher for exchange credentials, built from `CREDENTIALS_ENCRYPTION_KEY`.
    /// `None` = credential-write endpoints return 503 rather than ever storing plaintext.
    pub credential_cipher: Option<Arc<CredentialCipher>>,
    /// In-memory candle history for the live chart page (`GET /v1/candles`).
    /// Populated by `exchange_handler::spawn_price_forwarder` as candles close.
    pub candle_history: CandleHistory,
    /// Hard-coded registry of historical REST candle sources, keyed by exchange
    /// name (e.g. `"coinex"`). Deliberately separate from `adapter_factories`:
    /// not every exchange has a public historical-kline endpoint (Tabdeal and
    /// Hitobit do not), so an exchange simply has no entry here rather than an
    /// `Unsupported` stub implementation.
    pub historical_sources: Arc<HashMap<String, Arc<dyn HistoricalCandleSource>>>,
    /// Hard-coded registry of top-market-by-volume sources, keyed by exchange
    /// name. Same sparsity rationale as `historical_sources` — only exchanges
    /// with a public ticker/volume endpoint get an entry.
    pub top_market_sources: Arc<HashMap<String, Arc<dyn TopMarketSource>>>,
}

impl AppState {
    /// Creates the broadcast sender used to fan out price ticks to WS clients.
    pub fn new_broadcaster() -> broadcast::Sender<String> {
        broadcast::channel(BROADCAST_CAPACITY).0
    }

    /// Creates an empty `CandleHistory` map.
    pub fn new_candle_history() -> CandleHistory {
        Arc::new(Mutex::new(HashMap::new()))
    }

    /// Appends a closed candle to its key's history, evicting the oldest
    /// entry once `CANDLE_HISTORY_CAPACITY` is exceeded.
    pub async fn push_candle_history(&self, candle: &CandlePayload) {
        let key = format!("{}:{}:{}", candle.exchange, candle.pair, candle.interval);
        let mut history = self.candle_history.lock().await;
        let bucket = history.entry(key).or_default();
        bucket.push_back(candle.clone());
        while bucket.len() > CANDLE_HISTORY_CAPACITY {
            bucket.pop_front();
        }
    }

    /// Returns up to `limit` most recent candles for the given key, oldest first.
    pub async fn recent_candles(
        &self,
        exchange: &str,
        pair: &str,
        interval: &str,
        limit: usize,
    ) -> Vec<CandlePayload> {
        let key = format!("{exchange}:{pair}:{interval}");
        let history = self.candle_history.lock().await;
        match history.get(&key) {
            Some(bucket) => {
                let skip = bucket.len().saturating_sub(limit);
                bucket.iter().skip(skip).cloned().collect()
            }
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candle(exchange: &str, pair: &str, interval: &str, close: u64) -> CandlePayload {
        CandlePayload {
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            interval: interval.to_string(),
            time: chrono::Utc::now(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
        }
    }

    fn state_with_history() -> AppState {
        AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(
                crate::exchange::registry::ExchangeRegistry::new(),
            )),
            adapter_factories: Arc::new(HashMap::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
            ticker_repository: None,
            running_strategies: Arc::new(Mutex::new(HashMap::new())),
            strategy_repository: None,
            signal_repository: None,
            order_adapters: Arc::new(RwLock::new(HashMap::new())),
            order_manager: None,
            python_strategy_repository: None,
            candle_repository: None,
            exchange_repository: None,
            user_repository: None,
            credential_repository: None,
            credential_cipher: None,
            candle_history: AppState::new_candle_history(),
            historical_sources: Arc::new(HashMap::new()),
            top_market_sources: Arc::new(HashMap::new()),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recent_candles_returns_empty_for_unknown_key() {
        let state = state_with_history();
        let result = state.recent_candles("tabdeal", "USDT/IRT", "1m", 10).await;
        assert!(result.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_candle_history_then_recent_candles_returns_it() {
        let state = state_with_history();
        state
            .push_candle_history(&sample_candle("tabdeal", "USDT/IRT", "1m", 100))
            .await;
        let result = state.recent_candles("tabdeal", "USDT/IRT", "1m", 10).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].close, 100);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recent_candles_respects_limit_keeping_the_newest() {
        let state = state_with_history();
        for close in 1..=5u64 {
            state
                .push_candle_history(&sample_candle("tabdeal", "USDT/IRT", "1m", close))
                .await;
        }
        let result = state.recent_candles("tabdeal", "USDT/IRT", "1m", 2).await;
        assert_eq!(
            result.iter().map(|c| c.close).collect::<Vec<_>>(),
            vec![4, 5]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_candle_history_evicts_oldest_past_capacity() {
        let state = state_with_history();
        for close in 0..(CANDLE_HISTORY_CAPACITY as u64 + 5) {
            state
                .push_candle_history(&sample_candle("tabdeal", "USDT/IRT", "1m", close))
                .await;
        }
        let result = state
            .recent_candles("tabdeal", "USDT/IRT", "1m", CANDLE_HISTORY_CAPACITY + 10)
            .await;
        assert_eq!(result.len(), CANDLE_HISTORY_CAPACITY);
        assert_eq!(
            result[0].close, 5,
            "oldest 5 entries must have been evicted"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_candle_history_keeps_separate_keys_independent() {
        let state = state_with_history();
        state
            .push_candle_history(&sample_candle("tabdeal", "USDT/IRT", "1m", 1))
            .await;
        state
            .push_candle_history(&sample_candle("hitobit", "USDT/IRT", "1m", 2))
            .await;
        let tabdeal = state.recent_candles("tabdeal", "USDT/IRT", "1m", 10).await;
        let hitobit = state.recent_candles("hitobit", "USDT/IRT", "1m", 10).await;
        assert_eq!(tabdeal.len(), 1);
        assert_eq!(hitobit.len(), 1);
    }
}
