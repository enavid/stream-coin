use std::collections::VecDeque;
use std::time::Duration;

use thiserror::Error;
use tokio::time::Instant;

#[derive(Debug, Error)]
pub enum CircuitBreakerError {
    #[error("circuit breaker tripped: {0} orders in {1}s reached — manual reset required")]
    Tripped(u32, u64),
}

/// In-memory sliding-window circuit breaker for order placement.
///
/// Tracks order timestamps in a rolling window. When the order count in the
/// window reaches `max_orders`, the breaker trips and blocks all subsequent
/// orders until manually reset by an admin.
pub struct CircuitBreaker {
    max_orders: u32,
    window: Duration,
    timestamps: VecDeque<Instant>,
    tripped: bool,
}

impl CircuitBreaker {
    pub fn new(max_orders: u32, window_secs: u64) -> Self {
        Self {
            max_orders,
            window: Duration::from_secs(window_secs),
            timestamps: VecDeque::new(),
            tripped: false,
        }
    }

    /// Records one order attempt. Returns `Err` if the breaker is already tripped
    /// or just tripped due to this call. On `Ok`, the order may proceed.
    pub fn record_order(&mut self) -> Result<(), CircuitBreakerError> {
        if self.tripped {
            return Err(CircuitBreakerError::Tripped(
                self.max_orders,
                self.window.as_secs(),
            ));
        }

        let now = Instant::now();
        self.timestamps
            .retain(|t| now.duration_since(*t) < self.window);
        self.timestamps.push_back(now);

        if self.timestamps.len() as u32 >= self.max_orders {
            self.tripped = true;
            tracing::error!(
                max_orders = self.max_orders,
                window_secs = self.window.as_secs(),
                "circuit breaker tripped — order placement halted until admin reset"
            );
            return Err(CircuitBreakerError::Tripped(
                self.max_orders,
                self.window.as_secs(),
            ));
        }

        Ok(())
    }

    /// Hydrates the latched trip state from persistent storage at startup (M9).
    /// When `tripped` is true the breaker blocks immediately — the rolling
    /// window stays empty because a tripped breaker rejects regardless of count.
    pub fn restore_tripped(&mut self, tripped: bool) {
        self.tripped = tripped;
        if tripped {
            tracing::warn!(
                "circuit breaker restored to TRIPPED from persisted state — \
                 order placement halted until admin reset"
            );
        }
    }

    /// Admin-only manual reset.
    pub fn reset(&mut self) {
        self.tripped = false;
        self.timestamps.clear();
        tracing::info!("circuit breaker reset by admin");
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_allows_orders_below_threshold() {
        let mut cb = CircuitBreaker::new(3, 60);
        assert!(cb.record_order().is_ok());
        assert!(cb.record_order().is_ok());
        assert!(!cb.is_tripped());
    }

    #[test]
    fn circuit_breaker_halts_after_threshold() {
        let mut cb = CircuitBreaker::new(3, 60);
        cb.record_order().unwrap();
        cb.record_order().unwrap();
        let result = cb.record_order();
        assert!(
            result.is_err(),
            "third order must trip the breaker (threshold = 3)"
        );
        assert!(cb.is_tripped());
    }

    #[test]
    fn circuit_breaker_halts_all_subsequent_orders_when_tripped() {
        let mut cb = CircuitBreaker::new(2, 60);
        cb.record_order().unwrap();
        let _ = cb.record_order(); // trips
        assert!(cb.record_order().is_err());
        assert!(cb.record_order().is_err());
    }

    #[test]
    fn circuit_breaker_requires_manual_reset() {
        let mut cb = CircuitBreaker::new(2, 60);
        cb.record_order().unwrap();
        let _ = cb.record_order(); // trips

        assert!(cb.is_tripped());
        assert!(cb.record_order().is_err(), "still blocked before reset");

        cb.reset();
        assert!(!cb.is_tripped());
        assert!(cb.record_order().is_ok(), "allowed after reset");
    }

    #[test]
    fn circuit_breaker_reset_clears_timestamp_window() {
        let mut cb = CircuitBreaker::new(3, 60);
        cb.record_order().unwrap();
        cb.record_order().unwrap();
        let _ = cb.record_order(); // trips
        cb.reset();
        // After reset, the window is clear — can place max_orders-1 before hitting again
        assert!(cb.record_order().is_ok());
        assert!(cb.record_order().is_ok());
    }

    #[test]
    fn circuit_breaker_tripped_error_mentions_threshold_and_window() {
        let mut cb = CircuitBreaker::new(5, 30);
        for _ in 0..4 {
            cb.record_order().unwrap();
        }
        let err = cb.record_order().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains('5'), "error must mention max_orders");
        assert!(msg.contains("30"), "error must mention window_secs");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn circuit_breaker_window_expires_allows_new_orders() {
        tokio::time::pause();
        let mut cb = CircuitBreaker::new(3, 60);

        cb.record_order().unwrap();
        cb.record_order().unwrap();

        // Advance past the 60-second window — previous timestamps should expire
        tokio::time::advance(std::time::Duration::from_secs(61)).await;

        // After expiry, 2 new orders fit within the fresh window
        cb.record_order().unwrap();
        assert!(
            cb.record_order().is_ok(),
            "orders after window expiry must not be counted against old timestamps"
        );
        assert!(!cb.is_tripped());
    }
}
