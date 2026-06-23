use crate::price::entity::Price;
use crate::strategy::entity::{Action, RiskRewardConfig, Signal};
use crate::strategy::port::Strategy;

pub struct SpreadThresholdStrategy {
    strategy_id: String,
    exchange: String,
    pair: String,
    threshold: u64,
    risk_reward: Option<RiskRewardConfig>,
}

impl SpreadThresholdStrategy {
    pub fn new(strategy_id: &str, exchange: &str, pair: &str, threshold: u64) -> Self {
        Self {
            strategy_id: strategy_id.to_string(),
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            threshold,
            risk_reward: None,
        }
    }

    /// Attaches a `RiskRewardConfig` so future `Buy` signals carry computed
    /// `stop_loss`/`take_profit` levels. Additive — strategies built without
    /// calling this keep emitting `None`/`None`, exactly as before.
    pub fn with_risk_reward(mut self, config: RiskRewardConfig) -> Self {
        self.risk_reward = Some(config);
        self
    }
}

impl Strategy for SpreadThresholdStrategy {
    fn strategy_id(&self) -> &str {
        &self.strategy_id
    }

    fn on_price(&self, price: &Price) -> Option<Signal> {
        if price.exchange.as_str() != self.exchange.as_str() || price.pair.to_string() != self.pair
        {
            return None;
        }
        if price.spread() > self.threshold {
            let (stop_loss, take_profit) = self
                .risk_reward
                .map(|rr| rr.compute(price.ask, &Action::Buy))
                .unwrap_or((None, None));
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: price.exchange.to_string(),
                pair: price.pair.to_string(),
                action: Action::Buy,
                confidence: 1.0,
                timestamp: price.timestamp,
                stop_loss,
                take_profit,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::exchange::entity::ExchangeId;
    use crate::price::entity::TradingPair;
    use crate::strategy::entity::Action;
    use crate::strategy::port::Strategy;

    fn make_price(bid: u64, ask: u64) -> Price {
        Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRT"),
            bid,
            ask,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn spread_threshold_emits_buy_above_threshold() {
        let strategy = SpreadThresholdStrategy::new("test-id", "tabdeal", "USDT/IRT", 1_000);
        let price = make_price(175_000, 177_500);
        let signal = strategy.on_price(&price);
        assert!(signal.is_some());
        assert_eq!(signal.unwrap().action, Action::Buy);
    }

    #[test]
    fn spread_threshold_returns_none_below_threshold() {
        let strategy = SpreadThresholdStrategy::new("test-id", "tabdeal", "USDT/IRT", 5_000);
        let price = make_price(175_000, 175_500);
        let signal = strategy.on_price(&price);
        assert!(signal.is_none());
    }

    #[test]
    fn spread_threshold_returns_none_at_exactly_threshold() {
        let strategy = SpreadThresholdStrategy::new("test-id", "tabdeal", "USDT/IRT", 500);
        // spread = 500, threshold = 500; condition is '>' not '>=' so must return None
        let price = make_price(175_000, 175_500);
        let signal = strategy.on_price(&price);
        assert!(
            signal.is_none(),
            "spread equal to threshold must not trigger a signal (condition is strict >)"
        );
    }

    #[test]
    fn spread_threshold_ignores_wrong_exchange() {
        let strategy = SpreadThresholdStrategy::new("test-id", "tabdeal", "USDT/IRT", 100);
        let mut price = make_price(175_000, 177_000);
        price.exchange = ExchangeId::new("hitobit");
        let signal = strategy.on_price(&price);
        assert!(
            signal.is_none(),
            "strategy must ignore prices from a different exchange"
        );
    }

    #[test]
    fn spread_threshold_ignores_wrong_pair() {
        let strategy = SpreadThresholdStrategy::new("test-id", "tabdeal", "USDT/IRT", 100);
        let mut price = make_price(175_000, 177_000);
        price.pair = TradingPair::new("BTC", "IRT");
        let signal = strategy.on_price(&price);
        assert!(
            signal.is_none(),
            "strategy must ignore prices for a different pair"
        );
    }

    #[test]
    fn spread_threshold_signal_fields_match_input() {
        let strategy = SpreadThresholdStrategy::new("my-id", "tabdeal", "USDT/IRT", 1_000);
        let price = make_price(175_000, 177_500);
        let signal = strategy
            .on_price(&price)
            .expect("spread > threshold must emit a signal");
        assert_eq!(signal.strategy_id, "my-id");
        assert_eq!(signal.exchange, "tabdeal");
        assert_eq!(signal.pair, "USDT/IRT");
        assert_eq!(signal.action, Action::Buy);
        assert_eq!(signal.confidence, 1.0);
    }

    #[test]
    fn spread_threshold_signal_has_no_stop_loss_or_take_profit_yet() {
        let strategy = SpreadThresholdStrategy::new("my-id", "tabdeal", "USDT/IRT", 1_000);
        let price = make_price(175_000, 177_500);
        let signal = strategy
            .on_price(&price)
            .expect("spread > threshold must emit a signal");
        assert!(signal.stop_loss.is_none());
        assert!(signal.take_profit.is_none());
    }

    #[test]
    fn spread_threshold_without_risk_reward_config_leaves_sl_tp_none() {
        let strategy = SpreadThresholdStrategy::new("my-id", "tabdeal", "USDT/IRT", 1_000);
        let price = make_price(175_000, 177_500);
        let signal = strategy.on_price(&price).unwrap();
        assert!(signal.stop_loss.is_none());
        assert!(signal.take_profit.is_none());
    }

    #[test]
    fn spread_threshold_with_risk_reward_config_sets_stop_and_target() {
        let strategy = SpreadThresholdStrategy::new("my-id", "tabdeal", "USDT/IRT", 1_000)
            .with_risk_reward(crate::strategy::entity::RiskRewardConfig {
                stop_pct: 0.02,
                target_rr: 2.0,
            });
        let price = make_price(175_000, 177_500);
        let signal = strategy.on_price(&price).unwrap();

        // entry = ask = 177_500; risk = 177_500 * 0.02 = 3_550
        assert_eq!(signal.stop_loss, Some(173_950));
        assert_eq!(signal.take_profit, Some(184_600));
    }

    #[test]
    fn spread_threshold_with_risk_reward_config_stop_is_below_entry_for_buy() {
        let strategy = SpreadThresholdStrategy::new("my-id", "tabdeal", "USDT/IRT", 1_000)
            .with_risk_reward(crate::strategy::entity::RiskRewardConfig {
                stop_pct: 0.01,
                target_rr: 1.5,
            });
        let price = make_price(175_000, 177_500);
        let signal = strategy.on_price(&price).unwrap();

        assert!(
            signal.stop_loss.unwrap() < price.ask,
            "stop must be below entry for a Buy"
        );
        assert!(
            signal.take_profit.unwrap() > price.ask,
            "target must be above entry for a Buy"
        );
    }
}
