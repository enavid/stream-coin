use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::candle::entity::{Candle, Interval};
use crate::exchange::entity::ExchangeId;
use crate::price::entity::TradingPair;

/// Errors produced by a `HistoricalCandleSource`.
///
/// Every variant must be classified as transient or permanent via
/// `is_transient()`, per the project's transient/permanent error rule —
/// callers must never retry a permanent error.
#[derive(Debug, Error)]
pub enum HistoricalCandleSourceError {
    /// Transient — network did not respond within the hard deadline.
    #[error("network timeout: {0}")]
    NetworkTimeout(String),

    /// Transient — exchange returned 5xx.
    #[error("server error: status={status}, body={body}")]
    ServerError { status: u16, body: String },

    /// Permanent — exchange rejected the request (4xx).
    #[error("client error: status={status}, body={body}")]
    ClientError { status: u16, body: String },

    /// Permanent — could not parse the exchange's response.
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl HistoricalCandleSourceError {
    /// Returns `true` for errors that are safe to retry.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::NetworkTimeout(_) | Self::ServerError { .. })
    }
}

/// Port for fetching historical OHLCV candles from an exchange's public REST
/// API. Separate from `ExchangeAdapter` (the live WS price feed) because not
/// every exchange exposes historical klines — Tabdeal and Hitobit do not
/// today, so forcing them to implement this trait would mean an `Unsupported`
/// stub on two of three adapters forever.
#[async_trait]
pub trait HistoricalCandleSource: Send + Sync {
    /// Returns the canonical identifier for the exchange this source serves.
    fn exchange_id(&self) -> ExchangeId;

    /// Fetches all candles for `pair`/`interval` in `[from, to]`, paginating
    /// internally if the exchange's API caps results per request.
    async fn fetch_klines(
        &self,
        pair: &TradingPair,
        interval: Interval,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Candle>, HistoricalCandleSourceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_candle_source_error_network_timeout_is_transient() {
        let err = HistoricalCandleSourceError::NetworkTimeout("timed out".to_string());
        assert!(err.is_transient());
    }

    #[test]
    fn historical_candle_source_error_server_error_is_transient() {
        let err = HistoricalCandleSourceError::ServerError {
            status: 503,
            body: "unavailable".to_string(),
        };
        assert!(err.is_transient(), "5xx must be transient");
    }

    #[test]
    fn historical_candle_source_error_client_error_is_not_transient() {
        let err = HistoricalCandleSourceError::ClientError {
            status: 400,
            body: "bad request".to_string(),
        };
        assert!(!err.is_transient(), "4xx must be permanent");
    }

    #[test]
    fn historical_candle_source_error_serialization_is_not_transient() {
        let err = HistoricalCandleSourceError::Serialization("bad json".to_string());
        assert!(!err.is_transient());
    }
}
