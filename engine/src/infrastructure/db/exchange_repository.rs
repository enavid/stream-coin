use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::exchange::registry::{ExchangeRecord, TradingPairRecord};

#[derive(Debug, Error)]
pub enum ExchangeRepositoryError {
    #[error("database error: {0}")]
    Database(String),

    /// `upsert_pair`'s `base`/`quote` symbol has no matching row in the
    /// canonical `assets` table (migration `0013`). Callers must seed the
    /// asset first — pairs are never created for unknown symbols.
    #[error("unknown asset symbol: {0}")]
    UnknownAsset(String),
}

#[async_trait]
pub trait ExchangeRepository: Send + Sync {
    /// Loads every known exchange and trading pair, regardless of enabled/active state.
    /// Callers filter for enabled/active themselves (mirrors `ExchangeRegistry`'s API).
    async fn load_all(
        &self,
    ) -> Result<(Vec<ExchangeRecord>, Vec<TradingPairRecord>), ExchangeRepositoryError>;

    /// Persists the enabled flag for an exchange. No-op if the exchange does not exist.
    async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), ExchangeRepositoryError>;

    /// Inserts or updates a trading pair, keyed by
    /// `(exchange_name, base, quote, market_type)`. No-op if the exchange
    /// does not exist. Idempotent — used by the top-market seeder.
    async fn upsert_pair(&self, record: &TradingPairRecord) -> Result<(), ExchangeRepositoryError>;
}

pub struct FakeExchangeRepository {
    exchanges: Mutex<Vec<ExchangeRecord>>,
    pairs: Mutex<Vec<TradingPairRecord>>,
}

impl FakeExchangeRepository {
    pub fn new_with(exchanges: Vec<ExchangeRecord>, pairs: Vec<TradingPairRecord>) -> Self {
        Self {
            exchanges: Mutex::new(exchanges),
            pairs: Mutex::new(pairs),
        }
    }
}

#[async_trait]
impl ExchangeRepository for FakeExchangeRepository {
    async fn load_all(
        &self,
    ) -> Result<(Vec<ExchangeRecord>, Vec<TradingPairRecord>), ExchangeRepositoryError> {
        Ok((
            self.exchanges.lock().await.clone(),
            self.pairs.lock().await.clone(),
        ))
    }

    async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), ExchangeRepositoryError> {
        let mut exchanges = self.exchanges.lock().await;
        if let Some(e) = exchanges.iter_mut().find(|e| e.name == name) {
            e.enabled = enabled;
        }
        Ok(())
    }

    async fn upsert_pair(&self, record: &TradingPairRecord) -> Result<(), ExchangeRepositoryError> {
        let exchanges = self.exchanges.lock().await;
        if !exchanges.iter().any(|e| e.name == record.exchange_name) {
            return Ok(());
        }
        drop(exchanges);

        let mut pairs = self.pairs.lock().await;
        match pairs.iter_mut().find(|p| {
            p.exchange_name == record.exchange_name
                && p.base == record.base
                && p.quote == record.quote
                && p.market_type == record.market_type
        }) {
            Some(existing) => *existing = record.clone(),
            None => pairs.push(record.clone()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::exchange_repository::FakeExchangeRepository;
    use crate::price::entity::MarketType;

    #[test]
    fn unknown_asset_error_message_includes_the_symbol() {
        let err = ExchangeRepositoryError::UnknownAsset("DOGE".to_string());

        assert_eq!(err.to_string(), "unknown asset symbol: DOGE");
    }

    fn tabdeal(enabled: bool) -> ExchangeRecord {
        ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_all_returns_seeded_exchanges_and_pairs() {
        let repo = FakeExchangeRepository::new_with(
            vec![tabdeal(true)],
            vec![TradingPairRecord {
                exchange_name: "tabdeal".to_string(),
                base: "USDT".to_string(),
                quote: "IRT".to_string(),
                market_type: MarketType::Spot,
                active: true,
            }],
        );

        let (exchanges, pairs) = repo.load_all().await.unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0].name, "tabdeal");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].base, "USDT");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_enabled_updates_existing_exchange() {
        let repo = FakeExchangeRepository::new_with(vec![tabdeal(false)], vec![]);

        repo.set_enabled("tabdeal", true).await.unwrap();

        let (exchanges, _) = repo.load_all().await.unwrap();
        assert!(exchanges[0].enabled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_enabled_is_noop_for_unknown_exchange() {
        let repo = FakeExchangeRepository::new_with(vec![], vec![]);

        let result = repo.set_enabled("unknown", true).await;
        assert!(result.is_ok());
    }

    fn pair(exchange: &str, base: &str, quote: &str, active: bool) -> TradingPairRecord {
        TradingPairRecord {
            exchange_name: exchange.to_string(),
            base: base.to_string(),
            quote: quote.to_string(),
            market_type: MarketType::Spot,
            active,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_pair_inserts_new_pair_for_known_exchange() {
        let repo = FakeExchangeRepository::new_with(vec![tabdeal(true)], vec![]);

        repo.upsert_pair(&pair("tabdeal", "BTC", "USDT", true))
            .await
            .unwrap();

        let (_, pairs) = repo.load_all().await.unwrap();
        assert_eq!(pairs.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_pair_is_noop_for_unknown_exchange() {
        let repo = FakeExchangeRepository::new_with(vec![], vec![]);

        repo.upsert_pair(&pair("coinex", "BTC", "USDT", true))
            .await
            .unwrap();

        let (_, pairs) = repo.load_all().await.unwrap();
        assert!(
            pairs.is_empty(),
            "unknown exchange must not create a pair row"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_pair_is_idempotent_on_rerun() {
        let repo = FakeExchangeRepository::new_with(vec![tabdeal(true)], vec![]);

        repo.upsert_pair(&pair("tabdeal", "BTC", "USDT", true))
            .await
            .unwrap();
        repo.upsert_pair(&pair("tabdeal", "BTC", "USDT", true))
            .await
            .unwrap();

        let (_, pairs) = repo.load_all().await.unwrap();
        assert_eq!(pairs.len(), 1, "re-upserting must not duplicate the row");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upsert_pair_updates_active_flag_on_matching_key() {
        let repo = FakeExchangeRepository::new_with(vec![tabdeal(true)], vec![]);

        repo.upsert_pair(&pair("tabdeal", "BTC", "USDT", false))
            .await
            .unwrap();
        repo.upsert_pair(&pair("tabdeal", "BTC", "USDT", true))
            .await
            .unwrap();

        let (_, pairs) = repo.load_all().await.unwrap();
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].active);
    }
}
