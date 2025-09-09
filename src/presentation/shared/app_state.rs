use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use sea_orm::{DatabaseConnection};
use redis::aio::MultiplexedConnection;
use rdkafka::producer::FutureProducer;
use crate::infrastructure::websocket::ws_client_trait::WebSocketClient;

pub type ClientKey = String;
pub type StreamKey = String;

#[derive(Clone)]
pub struct AppState {
    // pub kafka: Arc<FutureProducer>,
    // pub db: Arc<DatabaseConnection>,
    // pub redis: Arc<tokio::sync::Mutex<MultiplexedConnection>>,
    pub clients: Arc<Mutex<HashMap<ClientKey, Arc<Mutex<Box<dyn WebSocketClient>>>>>>
}
