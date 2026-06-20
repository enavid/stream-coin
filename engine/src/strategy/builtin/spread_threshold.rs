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
}
