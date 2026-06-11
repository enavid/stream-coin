use std::collections::HashMap;
use std::sync::Arc;

use rdkafka::producer::FutureProducer;
use redis::aio::MultiplexedConnection;
use tokio::sync::Mutex;

use crate::infrastructure::websocket::ws_client_trait::WebSocketClient;

pub type ClientKey = String;
pub type WsClient = Arc<Mutex<Box<dyn WebSocketClient>>>;
pub type ClientMap = Arc<Mutex<HashMap<ClientKey, WsClient>>>;

#[derive(Clone)]
pub struct AppState {
    pub kafka: Option<Arc<FutureProducer>>,
    pub redis: Option<MultiplexedConnection>,
    pub clients: ClientMap,
}
