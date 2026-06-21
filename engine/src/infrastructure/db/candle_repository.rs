use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::candle::entity::CandlePayload;

#[derive(Debug, Error)]
pub enum CandleRepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

/// Port for reading historical candle data.
///
/// The production implementation queries the TimescaleDB `candles` hypertable.
/// Tests use `FakeCandleRepository`.
#[async_trait]
pub trait CandleRepository: Send + Sync {
    async fn list_candles(
        &self,
        exchange: &str,
        pair: &str,
        interval: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CandlePayload>, CandleRepositoryError>;
}

/// In-memory candle store for tests.  Filters by time range; ignores
/// exchange/pair/interval so a single `FakeCandleRepository` can cover
/// all combinations in a test suite.
pub struct FakeCandleRepository {
    candles: Vec<CandlePayload>,
}

impl FakeCandleRepository {
    pub fn new(candles: Vec<CandlePayload>) -> Self {
        Self { candles }
    }
}

#[async_trait]
impl CandleRepository for FakeCandleRepository {
    async fn list_candles(
        &self,
        _exchange: &str,
        _pair: &str,
        _interval: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CandlePayload>, CandleRepositoryError> {
        Ok(self
            .candles
            .iter()
            .filter(|c| c.time >= from && c.time <= to)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn sample_candle(time_secs: i64) -> CandlePayload {
        CandlePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: Utc.timestamp_opt(time_secs, 0).unwrap(),
            open: 100_000,
            high: 101_000,
            low: 99_000,
            close: 100_000,
            volume: 10,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_candle_repository_filters_by_time_range() {
        let repo = FakeCandleRepository::new(vec![
            sample_candle(1000),
            sample_candle(2000),
            sample_candle(3000),
        ]);
        let from = Utc.timestamp_opt(1500, 0).unwrap();
        let to = Utc.timestamp_opt(2500, 0).unwrap();
        let result = repo
            .list_candles("tabdeal", "USDT/IRT", "1m", from, to)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].time, Utc.timestamp_opt(2000, 0).unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_candle_repository_returns_all_in_inclusive_range() {
        let repo = FakeCandleRepository::new(vec![sample_candle(1000), sample_candle(2000)]);
        let from = Utc.timestamp_opt(1000, 0).unwrap();
        let to = Utc.timestamp_opt(2000, 0).unwrap();
        let result = repo
            .list_candles("tabdeal", "USDT/IRT", "1m", from, to)
            .await
            .unwrap();
        assert_eq!(result.len(), 2, "range must be inclusive on both ends");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_candle_repository_returns_empty_when_no_match() {
        let repo = FakeCandleRepository::new(vec![sample_candle(5000)]);
        let from = Utc.timestamp_opt(1000, 0).unwrap();
        let to = Utc.timestamp_opt(2000, 0).unwrap();
        let result = repo
            .list_candles("tabdeal", "USDT/IRT", "1m", from, to)
            .await
            .unwrap();
        assert!(result.is_empty());
    }
}
