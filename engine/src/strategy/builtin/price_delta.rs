use std::collections::VecDeque;
use std::sync::Mutex;

use crate::candle::entity::Candle;
use crate::strategy::entity::{Action, Signal};
use crate::strategy::port::Strategy;

pub struct PriceDeltaStrategy {
    strategy_id: String,
    exchange: String,
    pair: String,
    window: usize,
    threshold: f64,
    history: Mutex<VecDeque<u64>>,
}

impl PriceDeltaStrategy {
    pub fn new(
        strategy_id: &str,
        exchange: &str,
        pair: &str,
        window: usize,
        threshold: f64,
    ) -> Self {
        Self {
            strategy_id: strategy_id.to_string(),
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            window,
            threshold,
            history: Mutex::new(VecDeque::with_capacity(window)),
        }
    }
}

impl Strategy for PriceDeltaStrategy {
    fn strategy_id(&self) -> &str {
        &self.strategy_id
    }

    fn on_candle(&self, candle: &Candle) -> Option<Signal> {
        if candle.exchange != self.exchange || candle.pair != self.pair {
            return None;
        }
        let mut history = match self.history.lock() {
            Ok(h) => h,
            Err(_) => return None,
        };
        if history.len() >= self.window {
            history.pop_front();
        }
        history.push_back(candle.close);

        if history.len() < self.window {
            return None;
        }

        let oldest = *history.front()?;
        let newest = *history.back()?;

        if oldest == 0 {
            return None;
        }

        let delta = (newest as f64 - oldest as f64) / oldest as f64;

        if delta > self.threshold {
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: candle.exchange.clone(),
                pair: candle.pair.clone(),
                action: Action::Buy,
                confidence: delta.min(1.0),
                timestamp: candle.time,
            })
        } else if delta < -self.threshold {
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: candle.exchange.clone(),
                pair: candle.pair.clone(),
                action: Action::Sell,
                confidence: (-delta).min(1.0),
                timestamp: candle.time,
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
    use crate::candle::entity::Interval;

    fn make_candle(exchange: &str, pair: &str, close: u64) -> Candle {
        Candle {
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            interval: Interval::OneMinute,
            time: Utc::now(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 0,
        }
    }

    #[test]
    fn price_delta_returns_none_before_window_full() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 3, 0.01);
        let candle = make_candle("tabdeal", "USDT/IRT", 100_000);
        // Only one tick — window of 3 not yet full
        assert!(strategy.on_candle(&candle).is_none());
        assert!(strategy.on_candle(&candle).is_none());
    }

    #[test]
    fn price_delta_emits_buy_on_upward_move() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.05);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 110_000))
            .expect("10% gain above 5% threshold must emit a signal");
        assert_eq!(signal.action, Action::Buy);
    }

    #[test]
    fn price_delta_emits_sell_on_downward_move() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.05);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 90_000))
            .expect("10% drop below -5% threshold must emit a signal");
        assert_eq!(signal.action, Action::Sell);
    }

    #[test]
    fn price_delta_returns_none_below_threshold() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.05);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        // 3% move, threshold 5% — must not trigger
        let signal = strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 103_000));
        assert!(
            signal.is_none(),
            "move within threshold must not emit a signal"
        );
    }

    #[test]
    fn price_delta_confidence_capped_at_one() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.01);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        // 200% gain — confidence must not exceed 1.0
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 300_000))
            .unwrap();
        assert!(
            signal.confidence <= 1.0,
            "confidence must be capped at 1.0, got {}",
            signal.confidence
        );
    }

    #[test]
    fn price_delta_ignores_wrong_exchange() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.01);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        // Same pair, different exchange
        let signal = strategy.on_candle(&make_candle("hitobit", "USDT/IRT", 200_000));
        assert!(
            signal.is_none(),
            "strategy must ignore candles from a different exchange"
        );
    }

    #[test]
    fn price_delta_ignores_wrong_pair() {
        let strategy = PriceDeltaStrategy::new("id", "tabdeal", "USDT/IRT", 2, 0.01);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        // Same exchange, different pair
        let signal = strategy.on_candle(&make_candle("tabdeal", "BTC/IRT", 200_000));
        assert!(
            signal.is_none(),
            "strategy must ignore candles for a different pair"
        );
    }

    #[test]
    fn price_delta_signal_fields_match_input() {
        let strategy = PriceDeltaStrategy::new("my-id", "tabdeal", "USDT/IRT", 2, 0.05);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 110_000))
            .unwrap();
        assert_eq!(signal.strategy_id, "my-id");
        assert_eq!(signal.exchange, "tabdeal");
        assert_eq!(signal.pair, "USDT/IRT");
    }
}
