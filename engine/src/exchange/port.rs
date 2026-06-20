use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::task::AbortHandle;

use crate::exchange::entity::ExchangeId;
use crate::price::entity::{Price, TradingPair};

#[derive(Debug, Error)]
pub enum ExchangeAdapterError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
}

/// Drives a single exchange's real-time price feed.
///
/// Each implementor connects to one exchange's WebSocket API, parses its
/// proprietary message format into the shared [`Price`] type, and forwards
/// prices over an `mpsc` channel. The returned [`AbortHandle`] stops the
/// feed without requiring a mutable reference to the adapter.
#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    /// Returns the canonical identifier for the exchange this adapter drives.
    fn exchange_id(&self) -> ExchangeId;

    /// Converts a canonical [`TradingPair`] to the exchange-specific stream
    /// symbol used in WebSocket subscription messages.
    /// e.g. `TradingPair("USDT","IRT")` → `"usdtirt"` for Tabdeal.
    fn symbol_for_pair(&self, pair: &TradingPair) -> String;

    /// Subscribes to price updates for `pair` and starts forwarding them on
    /// `tx`. Returns an [`AbortHandle`] that stops the subscription when
    /// called; the adapter's background task exits cleanly on abort or when
    /// `tx` is dropped.
    async fn subscribe(
        &self,
        pair: &TradingPair,
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

        fn symbol_for_pair(&self, pair: &TradingPair) -> String {
            format!("{}{}", pair.base, pair.quote).to_lowercase()
        }

        async fn subscribe(
            &self,
            _pair: &TradingPair,
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

    #[tokio::test(flavor = "current_thread")]
    async fn exchange_adapter_returns_correct_exchange_id() {
        let adapter = MockAdapter;
        assert_eq!(adapter.exchange_id().to_string(), "mock");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exchange_adapter_subscribe_delivers_price_to_channel() {
        let adapter = MockAdapter;
        let (tx, mut rx) = mpsc::channel(1);

        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

        let price = rx.recv().await.unwrap();
        assert_eq!(price.bid, 100);
        assert_eq!(price.ask, 101);
        handle.abort();
    }
}
