use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct AssetRecord {
    pub id: i32,
    pub symbol: String,
    pub display_name: String,
    pub decimals: i16,
    pub icon_url: Option<String>,
    pub active: bool,
}

#[derive(Debug, Error)]
pub enum AssetRepositoryError {
    #[error("database error: {0}")]
    Database(String),
}

#[async_trait]
pub trait AssetRepository: Send + Sync {
    /// Loads every known asset, regardless of active state. Callers filter
    /// for active themselves (mirrors `ExchangeRegistry`'s API).
    async fn list_all(&self) -> Result<Vec<AssetRecord>, AssetRepositoryError>;

    /// Looks up a single asset by its canonical symbol (case-sensitive —
    /// symbols are stored uppercase, callers must match that).
    async fn find_by_symbol(
        &self,
        symbol: &str,
    ) -> Result<Option<AssetRecord>, AssetRepositoryError>;
}

pub struct FakeAssetRepository {
    assets: Mutex<Vec<AssetRecord>>,
}

impl FakeAssetRepository {
    pub fn new_with(assets: Vec<AssetRecord>) -> Self {
        Self {
            assets: Mutex::new(assets),
        }
    }
}

#[async_trait]
impl AssetRepository for FakeAssetRepository {
    async fn list_all(&self) -> Result<Vec<AssetRecord>, AssetRepositoryError> {
        Ok(self.assets.lock().await.clone())
    }

    async fn find_by_symbol(
        &self,
        symbol: &str,
    ) -> Result<Option<AssetRecord>, AssetRepositoryError> {
        Ok(self
            .assets
            .lock()
            .await
            .iter()
            .find(|a| a.symbol == symbol)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(symbol: &str, active: bool) -> AssetRecord {
        AssetRecord {
            id: 1,
            symbol: symbol.to_string(),
            display_name: symbol.to_string(),
            decimals: 8,
            icon_url: None,
            active,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_asset_repository_list_all_returns_seeded_assets() {
        let repo = FakeAssetRepository::new_with(vec![asset("BTC", true), asset("USDT", true)]);

        let assets = repo.list_all().await.unwrap();

        assert_eq!(assets.len(), 2);
        assert!(assets.iter().any(|a| a.symbol == "BTC"));
        assert!(assets.iter().any(|a| a.symbol == "USDT"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_asset_repository_find_by_symbol_returns_none_for_unknown_symbol() {
        let repo = FakeAssetRepository::new_with(vec![asset("BTC", true)]);

        let found = repo.find_by_symbol("DOGE").await.unwrap();

        assert!(found.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_asset_repository_find_by_symbol_returns_matching_asset() {
        let repo = FakeAssetRepository::new_with(vec![asset("BTC", true), asset("USDT", true)]);

        let found = repo.find_by_symbol("USDT").await.unwrap();

        assert_eq!(found.unwrap().symbol, "USDT");
    }
}
