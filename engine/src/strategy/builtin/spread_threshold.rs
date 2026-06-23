use crate::price::entity::Price;
use crate::strategy::entity::{Action, Signal};
use crate::strategy::port::Strategy;

pub struct SpreadThresholdStrategy {
    strategy_id: String,
    exchange: String,
    pair: String,
    threshold: u64,
}

impl SpreadThresholdStrategy {
    pub fn new(strategy_id: &str, exchange: &str, pair: &str, threshold: u64) -> Self {
        Self {
            strategy_id: strategy_id.to_string(),
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            threshold,
        }
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
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: price.exchange.to_string(),
                pair: price.pair.to_string(),
                action: Action::Buy,
                confidence: 1.0,
                timestamp: price.timestamp,
                stop_loss: None,
                take_profit: None,
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
}
