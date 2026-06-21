use std::collections::HashMap;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::AbortHandle;

use crate::exchange::port::ExchangeAdapter;
use crate::exchange::registry::ExchangeRegistry;
use crate::infrastructure::db::candle_repository::CandleRepository;
use crate::infrastructure::db::exchange_repository::ExchangeRepository;
use crate::infrastructure::db::python_strategy_repository::PythonStrategyRepository;
use crate::infrastructure::db::signal_repository::SignalRepository;
use crate::infrastructure::db::strategy_repository::StrategyRepository;
use crate::infrastructure::db::ticker_repository::TickerRepository;
use crate::kafka::port::MessagePublisher;
use crate::order::manager::OrderManager;
use crate::order::port::OrderAdapter;

pub type ClientKey = String;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, AbortHandle>>>;

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
    /// Admin credentials for `POST /v1/auth/token`.
    /// `None` = login endpoint disabled (server runs without a configured admin account).
    pub admin_credentials: Option<Arc<(String, String)>>,
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
}

impl AppState {
    /// Creates the broadcast sender used to fan out price ticks to WS clients.
    pub fn new_broadcaster() -> broadcast::Sender<String> {
        broadcast::channel(BROADCAST_CAPACITY).0
    }
}
