use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::candle::entity::{Candle, CandlePayload};

#[derive(Debug, Error)]
pub enum CandleRepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

/// Port for reading and persisting historical candle data.
///
/// The production implementation (`PostgresCandleRepository`) queries and
/// writes the TimescaleDB `candles` hypertable. Tests use `FakeCandleRepository`.
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

    /// Inserts or updates candles, keyed by `(exchange, pair, interval, time)`.
    /// Idempotent: re-upserting the same key updates that row in place rather
    /// than creating a duplicate — safe for both the live aggregator's
    /// closed-candle writes and a backfill re-run over an overlapping range.
    async fn upsert_candles(&self, candles: &[Candle]) -> Result<(), CandleRepositoryError>;
}

/// In-memory candle store for tests. `list_candles` filters by time range;
/// ignores exchange/pair/interval so a single `FakeCandleRepository` can
/// cover all combinations in a test suite. `upsert_candles` replicates the
/// production `ON CONFLICT ... DO UPDATE` semantics so callers can assert
/// idempotency without a real database.
#[derive(Default)]
pub struct FakeCandleRepository {
    candles: std::sync::Mutex<Vec<CandlePayload>>,
}

impl FakeCandleRepository {
    pub fn new(candles: Vec<CandlePayload>) -> Self {
        Self {
            candles: std::sync::Mutex::new(candles),
        }
    }

    pub fn snapshot(&self) -> Vec<CandlePayload> {
        self.candles.lock().unwrap().clone()
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
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.time >= from && c.time <= to)
            .cloned()
            .collect())
    }

    async fn upsert_candles(&self, candles: &[Candle]) -> Result<(), CandleRepositoryError> {
        let mut store = self.candles.lock().unwrap();
        for candle in candles {
            let payload = CandlePayload::from(candle);
            match store.iter_mut().find(|c| {
                c.exchange == payload.exchange
                    && c.pair == payload.pair
                    && c.interval == payload.interval
                    && c.time == payload.time
            }) {
                Some(existing) => *existing = payload,
                None => store.push(payload),
            }
        }
        Ok(())
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

    fn candle_entity(
        exchange: &str,
        pair: &str,
        interval: &str,
        time_secs: i64,
        close: u64,
    ) -> Candle {
        Candle {
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            interval: match interval {
                "5m" => crate::candle::entity::Interval::FiveMinutes,
                "15m" => crate::candle::entity::Interval::FifteenMinutes,
                "1h" => crate::candle::entity::Interval::OneHour,
                _ => crate::candle::entity::Interval::OneMinute,
            },
            time: Utc.timestamp_opt(time_secs, 0).unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
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

    // --- upsert_candles ---

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_is_idempotent_on_duplicate_time() {
        let repo = FakeCandleRepository::default();
        let candle = candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100);

        repo.upsert_candles(std::slice::from_ref(&candle))
            .await
            .unwrap();
        repo.upsert_candles(&[candle]).await.unwrap();

        assert_eq!(
            repo.snapshot().len(),
            1,
            "re-upserting the same key must not create a duplicate row"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_updates_close_when_time_matches_existing_row() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100)])
            .await
            .unwrap();
        repo.upsert_candles(&[candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 200)])
            .await
            .unwrap();

        let snapshot = repo.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].close, 200, "matching key must update in place");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_keeps_distinct_exchanges_separate() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[
            candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100),
            candle_entity("hitobit", "USDT/IRT", "1m", 1000, 200),
        ])
        .await
        .unwrap();

        assert_eq!(
            repo.snapshot().len(),
            2,
            "same time, different exchange must not collide"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_keeps_distinct_intervals_separate() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[
            candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100),
            candle_entity("tabdeal", "USDT/IRT", "5m", 1000, 200),
        ])
        .await
        .unwrap();

        assert_eq!(
            repo.snapshot().len(),
            2,
            "same time, different interval must not collide"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_persists_multiple_distinct_times_in_one_call() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[
            candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100),
            candle_entity("tabdeal", "USDT/IRT", "1m", 2000, 200),
            candle_entity("tabdeal", "USDT/IRT", "1m", 3000, 300),
        ])
        .await
        .unwrap();

        assert_eq!(repo.snapshot().len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_candles_empty_slice_is_a_no_op() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[]).await.unwrap();
        assert!(repo.snapshot().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upserted_candle_is_visible_through_list_candles() {
        let repo = FakeCandleRepository::default();
        repo.upsert_candles(&[candle_entity("tabdeal", "USDT/IRT", "1m", 1000, 100)])
            .await
            .unwrap();

        let from = Utc.timestamp_opt(0, 0).unwrap();
        let to = Utc.timestamp_opt(2000, 0).unwrap();
        let result = repo
            .list_candles("tabdeal", "USDT/IRT", "1m", from, to)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].close, 100);
    }
}
