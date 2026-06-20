use rust_decimal::Decimal;

/// Safety configuration applied to every order that passes through the Order Manager.
///
/// Loaded from the `safety_config` DB table at startup; defaults to `dry_run = true`
/// so no real orders are placed until an operator explicitly disables dry-run.
#[derive(Debug, Clone)]
pub struct SafetyConfig {
    /// Maximum total open quantity per pair. Orders that would push open quantity
    /// above this limit are rejected before reaching the exchange.
    pub max_position_size: Decimal,
    /// Number of orders within `circuit_breaker_window_secs` that trips the circuit breaker.
    pub circuit_breaker_max_orders: u32,
    /// Rolling window in seconds used by the circuit breaker.
    pub circuit_breaker_window_secs: u64,
    /// Signals with confidence below this value are silently dropped.
    pub min_confidence: f64,
    /// When `true`, all safety checks run but `place_order` is never called.
    /// Persisted as `"dry_run"` status so the full audit trail exists.
    pub dry_run: bool,
    /// Default order size (market orders from signals use this quantity).
    pub default_order_quantity: Decimal,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            max_position_size: Decimal::new(1_000_000, 0),
            circuit_breaker_max_orders: 10,
            circuit_breaker_window_secs: 60,
            min_confidence: 0.7,
            dry_run: true,
            default_order_quantity: Decimal::new(100, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_config_default_has_dry_run_enabled() {
        let cfg = SafetyConfig::default();
        assert!(
            cfg.dry_run,
            "default config must have dry_run = true for safety"
        );
    }

    #[test]
    fn safety_config_default_min_confidence_is_0_7() {
        let cfg = SafetyConfig::default();
        assert!((cfg.min_confidence - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn safety_config_default_max_position_size_is_nonzero() {
        let cfg = SafetyConfig::default();
        assert!(cfg.max_position_size > Decimal::ZERO);
    }
}
