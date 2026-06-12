use std::collections::HashMap;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

use crate::exchange::port::ExchangeAdapter;
use crate::kafka::port::MessagePublisher;
use crate::ticker::port::TickerRepository;

pub type ClientKey = String;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, AbortHandle>>>;

#[derive(Clone)]
pub struct AppState {
    pub redis: Option<MultiplexedConnection>,
    pub ticker_repository: Option<Arc<dyn TickerRepository>>,
    pub exchange_adapters: Arc<HashMap<String, Arc<dyn ExchangeAdapter>>>,
    pub clients: ClientMap,
    pub publisher: Option<Arc<dyn MessagePublisher>>,
}
