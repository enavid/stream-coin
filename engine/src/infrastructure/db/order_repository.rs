use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct OrderRecord {
    pub id: Option<i64>,
    pub exchange: String,
    pub pair: String,
    pub side: String,
    pub order_type: String,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub status: String,
    pub exchange_order_id: Option<String>,
    pub client_order_id: String,
    pub strategy_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum OrderRepositoryError {
    #[error("database error: {0}")]
    Database(String),
    #[error("order not found: client_order_id={0}")]
    NotFound(String),
}

#[async_trait]
pub trait OrderRepository: Send + Sync {
    /// Persists a new order record. Returns the assigned row `id`.
    async fn insert(&self, record: &OrderRecord) -> Result<i64, OrderRepositoryError>;

    /// Updates the status (and optional exchange_order_id / fill_price) for an order
    /// identified by `client_order_id`.
    async fn update_status(
        &self,
        client_order_id: &str,
        status: &str,
        exchange_order_id: Option<&str>,
        fill_price: Option<Decimal>,
    ) -> Result<(), OrderRepositoryError>;

    /// Returns total open quantity for the given exchange + pair.
    /// Used by the Order Manager to enforce position limits.
    async fn get_open_quantity(
        &self,
        exchange: &str,
        pair: &str,
    ) -> Result<Decimal, OrderRepositoryError>;

    /// Fetches a single order by its client-assigned idempotency key.
    async fn get_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Result<OrderRecord, OrderRepositoryError>;

    /// Lists orders, optionally filtered by exchange and/or pair.
    async fn list(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<OrderRecord>, OrderRepositoryError>;
}

// ---------------------------------------------------------------------------
// In-memory fake — used in unit and integration tests

pub struct FakeOrderRepository {
    records: Mutex<Vec<OrderRecord>>,
}

impl Default for FakeOrderRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeOrderRepository {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(vec![]),
        }
    }

    pub async fn all_records(&self) -> Vec<OrderRecord> {
        self.records.lock().await.clone()
    }
}

#[async_trait]
impl OrderRepository for FakeOrderRepository {
    async fn insert(&self, record: &OrderRecord) -> Result<i64, OrderRepositoryError> {
        let mut recs = self.records.lock().await;
        let id = (recs.len() + 1) as i64;
        let mut r = record.clone();
        r.id = Some(id);
        recs.push(r);
        Ok(id)
    }

    async fn update_status(
        &self,
        client_order_id: &str,
        status: &str,
        exchange_order_id: Option<&str>,
        fill_price: Option<Decimal>,
    ) -> Result<(), OrderRepositoryError> {
        let _ = fill_price;
        let mut recs = self.records.lock().await;
        let record = recs
            .iter_mut()
            .find(|r| r.client_order_id == client_order_id)
            .ok_or_else(|| OrderRepositoryError::NotFound(client_order_id.to_string()))?;
        record.status = status.to_string();
        if let Some(eid) = exchange_order_id {
            record.exchange_order_id = Some(eid.to_string());
        }
        record.updated_at = Utc::now();
        Ok(())
    }

    async fn get_open_quantity(
        &self,
        exchange: &str,
        pair: &str,
    ) -> Result<Decimal, OrderRepositoryError> {
        let recs = self.records.lock().await;
        let total = recs
            .iter()
            .filter(|r| r.exchange == exchange && r.pair == pair && r.status == "open")
            .map(|r| r.quantity)
            .fold(Decimal::ZERO, |acc, q| acc + q);
        Ok(total)
    }

    async fn get_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Result<OrderRecord, OrderRepositoryError> {
        self.records
            .lock()
            .await
            .iter()
            .find(|r| r.client_order_id == client_order_id)
            .cloned()
            .ok_or_else(|| OrderRepositoryError::NotFound(client_order_id.to_string()))
    }

    async fn list(
        &self,
        exchange: Option<&str>,
        pair: Option<&str>,
    ) -> Result<Vec<OrderRecord>, OrderRepositoryError> {
        let recs = self.records.lock().await;
        Ok(recs
            .iter()
            .filter(|r| {
                exchange.is_none_or(|e| r.exchange == e) && pair.is_none_or(|p| r.pair == p)
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn make_record(client_order_id: &str, status: &str, quantity: Decimal) -> OrderRecord {
        let now = Utc::now();
        OrderRecord {
            id: None,
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: "buy".to_string(),
            order_type: "market".to_string(),
            quantity,
            price: None,
            status: status.to_string(),
            exchange_order_id: None,
            client_order_id: client_order_id.to_string(),
            strategy_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_insert_assigns_sequential_ids() {
        let repo = FakeOrderRepository::new();
        let id1 = repo
            .insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let id2 = repo
            .insert(&make_record("uuid-2", "open", Decimal::new(200, 0)))
            .await
            .unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_changes_status() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.update_status("uuid-1", "filled", Some("exch-001"), None)
            .await
            .unwrap();
        let recs = repo.all_records().await;
        assert_eq!(recs[0].status, "filled");
        assert_eq!(recs[0].exchange_order_id.as_deref(), Some("exch-001"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_update_status_unknown_id_returns_not_found() {
        let repo = FakeOrderRepository::new();
        let result = repo
            .update_status("nonexistent", "filled", None, None)
            .await;
        assert!(matches!(result, Err(OrderRepositoryError::NotFound(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_open_quantity_sums_only_open_orders() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-2", "open", Decimal::new(200, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-3", "filled", Decimal::new(50, 0)))
            .await
            .unwrap();
        let qty = repo.get_open_quantity("tabdeal", "USDT/IRT").await.unwrap();
        assert_eq!(
            qty,
            Decimal::new(300, 0),
            "only open orders count toward position"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_open_quantity_filters_by_pair() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let qty = repo.get_open_quantity("tabdeal", "BTC/IRT").await.unwrap();
        assert_eq!(qty, Decimal::ZERO, "different pair must not count");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_list_returns_all_without_filter() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        repo.insert(&make_record("uuid-2", "filled", Decimal::new(200, 0)))
            .await
            .unwrap();
        let all = repo.list(None, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_by_client_order_id_returns_record() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-target", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let rec = repo.get_by_client_order_id("uuid-target").await.unwrap();
        assert_eq!(rec.client_order_id, "uuid-target");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_get_by_client_order_id_returns_not_found() {
        let repo = FakeOrderRepository::new();
        let result = repo.get_by_client_order_id("nonexistent").await;
        assert!(matches!(result, Err(OrderRepositoryError::NotFound(_))));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fake_order_repository_list_filters_by_exchange() {
        let repo = FakeOrderRepository::new();
        repo.insert(&make_record("uuid-1", "open", Decimal::new(100, 0)))
            .await
            .unwrap();
        let results = repo.list(Some("hitobit"), None).await.unwrap();
        assert!(
            results.is_empty(),
            "different exchange must be filtered out"
        );
    }
}
