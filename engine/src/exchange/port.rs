use std::fmt;

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use tokio::task::AbortHandle;

use crate::exchange::entity::ExchangeId;
use crate::price::entity::Price;

#[derive(Debug)]
pub enum ExchangeAdapterError {
    ConnectionFailed(String),
}

impl fmt::Display for ExchangeAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExchangeAdapterError::ConnectionFailed(msg) => write!(f, "connection failed: {}", msg),
        }
    }
}

#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    fn exchange_id(&self) -> ExchangeId;
    async fn subscribe(
        &self,
        symbol: &str,
        tx: Sender<Price>,
    ) -> Result<AbortHandle, ExchangeAdapterError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tokio::sync::mpsc;

    use super::*;
    use crate::price::entity::{Price, TradingPair};

    struct MockAdapter;

    #[async_trait]
    impl ExchangeAdapter for MockAdapter {
        fn exchange_id(&self) -> ExchangeId {
            ExchangeId::new("mock")
        }

        async fn subscribe(
            &self,
            _symbol: &str,
            tx: Sender<Price>,
        ) -> Result<AbortHandle, ExchangeAdapterError> {
            let handle = tokio::spawn(async move {
                let _ = tx
                    .send(Price {
                        exchange: ExchangeId::new("mock"),
                        pair: TradingPair::new("USDT", "IRT"),
                        bid: 100,
                        ask: 101,
                        timestamp: Utc::now(),
                    })
                    .await;
            });
            Ok(handle.abort_handle())
        }
    }

    #[tokio::test]
    async fn exchange_adapter_returns_correct_exchange_id() {
        let adapter = MockAdapter;
        assert_eq!(adapter.exchange_id().to_string(), "mock");
    }

    #[tokio::test]
    async fn exchange_adapter_subscribe_delivers_price_to_channel() {
        let adapter = MockAdapter;
        let (tx, mut rx) = mpsc::channel(1);

        let handle = adapter.subscribe("USDT", tx).await.unwrap();

        let price = rx.recv().await.unwrap();
        assert_eq!(price.bid, 100);
        assert_eq!(price.ask, 101);
        handle.abort();
    }
}
