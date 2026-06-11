use std::collections::HashMap;
use std::sync::Arc;

use redis::aio::MultiplexedConnection;
use tokio::sync::Mutex;

use crate::infrastructure::websocket::ws_client_trait::WebSocketClient;
use crate::ticker::port::TickerRepository;

pub type ClientKey = String;
pub type WsClient = Arc<Mutex<Box<dyn WebSocketClient>>>;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, WsClient>>>;

#[derive(Clone)]
pub struct AppState {
    pub redis: Option<MultiplexedConnection>,
    pub ticker_repository: Option<Arc<dyn TickerRepository>>,
    pub clients: ClientMap,
}
