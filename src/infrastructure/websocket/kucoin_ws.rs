use async_trait::async_trait;

pub struct ExchangeClient {
    exchange_name: String,
    symbols: Vec<String>,
}

impl ExchangeClient {
    pub fn new(exchange_name: String, symbols: Vec<String>) -> Self {
        Self { exchange_name, symbols }
    }

    pub async fn connect(&self) -> Result<(), String> {
        println!("🔗 Connecting to WebSocket for exchange {} with symbols {:?}", self.exchange_name, self.symbols);
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        println!("❌ Disconnecting from exchange {}", self.exchange_name);
        Ok(())
    }
}
