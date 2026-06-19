use std::collections::HashMap;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

use crate::exchange::port::ExchangeAdapter;
use crate::kafka::port::MessagePublisher;

pub type ClientKey = String;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, AbortHandle>>>;

/// Capacity of the price broadcast channel; lagging WS clients drop oldest
/// messages rather than blocking publishers.
pub const BROADCAST_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub redis: Option<MultiplexedConnection>,
    pub exchange_adapters: Arc<HashMap<String, Arc<dyn ExchangeAdapter>>>,
    pub clients: ClientMap,
    pub publisher: Option<Arc<dyn MessagePublisher>>,
    pub broadcaster: broadcast::Sender<String>,
}

impl AppState {
    pub fn new_broadcaster() -> broadcast::Sender<String> {
        broadcast::channel(BROADCAST_CAPACITY).0
    }
}
