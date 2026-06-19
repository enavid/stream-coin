use std::collections::HashMap;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::AbortHandle;

use crate::exchange::port::ExchangeAdapter;
use crate::exchange::registry::ExchangeRegistry;
use crate::kafka::port::MessagePublisher;

pub type ClientKey = String;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, AbortHandle>>>;

/// Factory function that constructs an `ExchangeAdapter` given a WebSocket URL.
pub type AdapterFactory = Arc<dyn Fn(&str) -> Arc<dyn ExchangeAdapter> + Send + Sync>;

/// Capacity of the price broadcast channel; lagging WS clients drop oldest
/// messages rather than blocking publishers.
pub const BROADCAST_CAPACITY: usize = 256;

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
}

impl AppState {
    /// Creates the broadcast sender used to fan out price ticks to WS clients.
    pub fn new_broadcaster() -> broadcast::Sender<String> {
        broadcast::channel(BROADCAST_CAPACITY).0
    }
}
