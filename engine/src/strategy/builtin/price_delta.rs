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
        let mut history = self.history.lock().expect("history mutex poisoned");
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
