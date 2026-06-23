use std::collections::VecDeque;
use std::sync::Mutex;

use crate::candle::entity::Candle;
use crate::strategy::entity::{Action, RiskRewardConfig, Signal};
use crate::strategy::port::Strategy;

pub struct PriceDeltaStrategy {
    strategy_id: String,
    exchange: String,
    pair: String,
    window: usize,
    threshold: f64,
    history: Mutex<VecDeque<u64>>,
    risk_reward: Option<RiskRewardConfig>,
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
            risk_reward: None,
        }
    }

    /// Attaches a `RiskRewardConfig` so future `Buy`/`Sell` signals carry
    /// computed `stop_loss`/`take_profit` levels. Additive — strategies
    /// built without calling this keep emitting `None`/`None`.
    pub fn with_risk_reward(mut self, config: RiskRewardConfig) -> Self {
        self.risk_reward = Some(config);
        self
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
            let (stop_loss, take_profit) = self
                .risk_reward
                .map(|rr| rr.compute(newest, &Action::Buy))
                .unwrap_or((None, None));
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: candle.exchange.clone(),
                pair: candle.pair.clone(),
                action: Action::Buy,
                confidence: delta.min(1.0),
                timestamp: candle.time,
                stop_loss,
                take_profit,
            })
        } else if delta < -self.threshold {
            let (stop_loss, take_profit) = self
                .risk_reward
                .map(|rr| rr.compute(newest, &Action::Sell))
                .unwrap_or((None, None));
            Some(Signal {
                strategy_id: self.strategy_id.clone(),
                exchange: candle.exchange.clone(),
                pair: candle.pair.clone(),
                action: Action::Sell,
                confidence: (-delta).min(1.0),
                timestamp: candle.time,
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

    #[test]
    fn price_delta_signal_has_no_stop_loss_or_take_profit_yet() {
        let strategy = PriceDeltaStrategy::new("my-id", "tabdeal", "USDT/IRT", 2, 0.05);
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 110_000))
            .unwrap();
        assert!(signal.stop_loss.is_none());
        assert!(signal.take_profit.is_none());
    }

    #[test]
    fn price_delta_with_risk_reward_config_sets_stop_and_target_on_buy() {
        let strategy = PriceDeltaStrategy::new("my-id", "tabdeal", "USDT/IRT", 2, 0.05)
            .with_risk_reward(crate::strategy::entity::RiskRewardConfig {
                stop_pct: 0.02,
                target_rr: 2.0,
            });
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 110_000))
            .unwrap();

        // entry = newest close = 110_000; risk = 110_000 * 0.02 = 2_200
        assert_eq!(signal.stop_loss, Some(107_800));
        assert_eq!(signal.take_profit, Some(114_400));
    }

    #[test]
    fn price_delta_with_risk_reward_config_sets_stop_and_target_on_sell() {
        let strategy = PriceDeltaStrategy::new("my-id", "tabdeal", "USDT/IRT", 2, 0.05)
            .with_risk_reward(crate::strategy::entity::RiskRewardConfig {
                stop_pct: 0.02,
                target_rr: 2.0,
            });
        strategy.on_candle(&make_candle("tabdeal", "USDT/IRT", 100_000));
        let signal = strategy
            .on_candle(&make_candle("tabdeal", "USDT/IRT", 90_000))
            .unwrap();

        // entry = newest close = 90_000; risk = 90_000 * 0.02 = 1_800
        assert_eq!(signal.stop_loss, Some(91_800));
        assert_eq!(signal.take_profit, Some(86_400));
    }
}
