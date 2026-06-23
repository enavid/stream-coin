use std::sync::Arc;

use crate::strategy::builtin::price_delta::PriceDeltaStrategy;
use crate::strategy::builtin::spread_threshold::SpreadThresholdStrategy;
use crate::strategy::entity::RiskRewardConfig;
use crate::strategy::port::Strategy;

/// Parses the optional `risk_reward: { stop_pct, target_rr }` object from a
/// strategy's `params`. Absent or malformed input is `None` — risk/reward is
/// additive, never a reason to reject an otherwise-valid strategy start.
fn parse_risk_reward(params: &serde_json::Value) -> Option<RiskRewardConfig> {
    let rr = params.get("risk_reward")?;
    let stop_pct = rr["stop_pct"].as_f64()?;
    let target_rr = rr["target_rr"].as_f64()?;
    Some(RiskRewardConfig {
        stop_pct,
        target_rr,
    })
}

pub fn build_strategy(
    strategy_id: &str,
    strategy_type: &str,
    exchange: &str,
    pair: &str,
    params: &serde_json::Value,
) -> Option<Arc<dyn Strategy>> {
    let risk_reward = parse_risk_reward(params);

    match strategy_type {
        "spread_threshold" => {
            let threshold = params["threshold"].as_u64()?;
            let mut strategy = SpreadThresholdStrategy::new(strategy_id, exchange, pair, threshold);
            if let Some(rr) = risk_reward {
                strategy = strategy.with_risk_reward(rr);
            }
            Some(Arc::new(strategy))
        }
        "price_delta" => {
            let window = params["window"].as_u64().unwrap_or(5) as usize;
            let threshold = params["threshold"].as_f64().unwrap_or(0.02);
            let mut strategy =
                PriceDeltaStrategy::new(strategy_id, exchange, pair, window, threshold);
            if let Some(rr) = risk_reward {
                strategy = strategy.with_risk_reward(rr);
            }
            Some(Arc::new(strategy))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_risk_reward_returns_none_when_absent() {
        let params = serde_json::json!({ "threshold": 1000 });
        assert!(parse_risk_reward(&params).is_none());
    }

    #[test]
    fn parse_risk_reward_extracts_stop_pct_and_target_rr() {
        let params = serde_json::json!({
            "threshold": 1000,
            "risk_reward": { "stop_pct": 0.02, "target_rr": 2.0 }
        });
        let rr = parse_risk_reward(&params).unwrap();
        assert_eq!(rr.stop_pct, 0.02);
        assert_eq!(rr.target_rr, 2.0);
    }

    #[test]
    fn parse_risk_reward_returns_none_when_missing_target_rr() {
        let params = serde_json::json!({
            "risk_reward": { "stop_pct": 0.02 }
        });
        assert!(parse_risk_reward(&params).is_none());
    }

    #[test]
    fn build_strategy_spread_threshold_without_risk_reward_param() {
        let params = serde_json::json!({ "threshold": 1000 });
        let strategy = build_strategy("id", "spread_threshold", "tabdeal", "USDT/IRT", &params);
        assert!(strategy.is_some());
    }

    #[test]
    fn build_strategy_spread_threshold_with_risk_reward_param() {
        let params = serde_json::json!({
            "threshold": 1000,
            "risk_reward": { "stop_pct": 0.02, "target_rr": 2.0 }
        });
        let strategy = build_strategy("id", "spread_threshold", "tabdeal", "USDT/IRT", &params);
        assert!(strategy.is_some());
    }

    #[test]
    fn build_strategy_price_delta_with_risk_reward_param() {
        let params = serde_json::json!({
            "window": 5,
            "threshold": 0.02,
            "risk_reward": { "stop_pct": 0.01, "target_rr": 1.5 }
        });
        let strategy = build_strategy("id", "price_delta", "tabdeal", "USDT/IRT", &params);
        assert!(strategy.is_some());
    }

    #[test]
    fn build_strategy_unknown_type_returns_none() {
        let params = serde_json::json!({});
        assert!(build_strategy("id", "unknown_type", "tabdeal", "USDT/IRT", &params).is_none());
    }

    #[test]
    fn build_strategy_spread_threshold_missing_threshold_returns_none() {
        let params = serde_json::json!({ "risk_reward": { "stop_pct": 0.02, "target_rr": 2.0 } });
        assert!(build_strategy("id", "spread_threshold", "tabdeal", "USDT/IRT", &params).is_none());
    }
}
