use std::sync::Arc;

use crate::strategy::builtin::price_delta::PriceDeltaStrategy;
use crate::strategy::builtin::spread_threshold::SpreadThresholdStrategy;
use crate::strategy::port::Strategy;

pub fn build_strategy(
    strategy_id: &str,
    strategy_type: &str,
    exchange: &str,
    pair: &str,
    params: &serde_json::Value,
) -> Option<Arc<dyn Strategy>> {
    match strategy_type {
        "spread_threshold" => {
            let threshold = params["threshold"].as_u64()?;
            Some(Arc::new(SpreadThresholdStrategy::new(
                strategy_id,
                exchange,
                pair,
                threshold,
            )))
        }
        "price_delta" => {
            let window = params["window"].as_u64().unwrap_or(5) as usize;
            let threshold = params["threshold"].as_f64().unwrap_or(0.02);
            Some(Arc::new(PriceDeltaStrategy::new(
                strategy_id,
                exchange,
                pair,
                window,
                threshold,
            )))
        }
        _ => None,
    }
}
