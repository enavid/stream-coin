//! Persistence port for the circuit-breaker trip state (M9).
//!
//! The in-memory [`CircuitBreaker`](super::circuit_breaker::CircuitBreaker) lives
//! per-process: a trip was lost on restart, so a crash-loop re-armed order
//! placement every time the engine came back up, and a trip on one instance was
//! invisible to the others. This port persists the single "tripped" bit to
//! shared storage (Postgres) so the trip:
//!   * survives a restart (hydrated at startup), and
//!   * is visible to every instance that reads it.
//!
//! Only the latched trip bit is persisted — the rolling order-count window stays
//! in memory (it is a short-lived rate signal, not safety-critical state). What
//! must never be silently lost is the *tripped* decision itself.

use async_trait::async_trait;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CircuitBreakerStoreError {
    #[error("circuit breaker store error: {0}")]
    Database(String),
}

#[async_trait]
pub trait CircuitBreakerStore: Send + Sync {
    /// Reads the persisted trip state. A fresh deployment with no row yet is
    /// "not tripped" (`Ok(false)`).
    async fn load_tripped(&self) -> Result<bool, CircuitBreakerStoreError>;

    /// Persists the trip state. Called when the breaker trips (`true`) and when
    /// an admin resets it (`false`).
    async fn set_tripped(&self, tripped: bool) -> Result<(), CircuitBreakerStoreError>;
}

/// In-memory store for tests. Behaves like the persistent one within a process:
/// a value written by one handle is visible to another handle sharing the same
/// `Arc`, which is exactly how a "restart" is simulated in unit tests (build a
/// second manager over the same store).
#[derive(Default)]
pub struct FakeCircuitBreakerStore {
    tripped: Mutex<bool>,
}

impl FakeCircuitBreakerStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CircuitBreakerStore for FakeCircuitBreakerStore {
    async fn load_tripped(&self) -> Result<bool, CircuitBreakerStoreError> {
        Ok(*self.tripped.lock().expect("lock"))
    }

    async fn set_tripped(&self, tripped: bool) -> Result<(), CircuitBreakerStoreError> {
        *self.tripped.lock().expect("lock") = tripped;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_store_defaults_to_not_tripped() {
        let store = FakeCircuitBreakerStore::new();
        assert!(!store.load_tripped().await.unwrap());
    }

    #[tokio::test]
    async fn fake_store_persists_tripped_state() {
        let store = FakeCircuitBreakerStore::new();
        store.set_tripped(true).await.unwrap();
        assert!(store.load_tripped().await.unwrap());
        store.set_tripped(false).await.unwrap();
        assert!(!store.load_tripped().await.unwrap());
    }
}
