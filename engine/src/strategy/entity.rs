use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Buy,
    Sell,
    Hold,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Buy => "buy",
            Action::Sell => "sell",
            Action::Hold => "hold",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: Action,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    /// `None` unless the emitting strategy was configured with a `RiskRewardConfig`.
    pub stop_loss: Option<u64>,
    /// `None` unless the emitting strategy was configured with a `RiskRewardConfig`.
    pub take_profit: Option<u64>,
}

/// Optional risk/reward parameters a built-in strategy can be configured
/// with. When present, `Buy`/`Sell` signals get computed `stop_loss`/
/// `take_profit` levels; when absent, behavior is unchanged from before
/// this existed (`stop_loss`/`take_profit` stay `None`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskRewardConfig {
    /// Stop distance from entry, as a fraction of entry price (e.g. `0.02` = 2%).
    pub stop_pct: f64,
    /// Target distance from entry, expressed as a multiple of the stop
    /// distance (e.g. `2.0` = take-profit is twice as far as the stop).
    pub target_rr: f64,
}

impl RiskRewardConfig {
    /// Computes `(stop_loss, take_profit)` for a fill at `entry_price`,
    /// signed by `action` — a stop sits below entry and a target above for a
    /// `Buy`, and the reverse for a `Sell`. Returns `(None, None)` for `Hold`
    /// (there is no position to protect).
    pub fn compute(&self, entry_price: u64, action: &Action) -> (Option<u64>, Option<u64>) {
        let entry = entry_price as f64;
        let risk = entry * self.stop_pct;
        match action {
            Action::Buy => {
                let stop = (entry - risk).max(0.0);
                let target = entry + risk * self.target_rr;
                (Some(stop.round() as u64), Some(target.round() as u64))
            }
            Action::Sell => {
                let stop = entry + risk;
                let target = (entry - risk * self.target_rr).max(0.0);
                (Some(stop.round() as u64), Some(target.round() as u64))
            }
            Action::Hold => (None, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_reward_config_buy_stop_below_entry_target_above() {
        let rr = RiskRewardConfig {
            stop_pct: 0.02,
            target_rr: 2.0,
        };
        let (stop, target) = rr.compute(100_000, &Action::Buy);
        assert_eq!(stop, Some(98_000));
        assert_eq!(target, Some(104_000));
    }

    #[test]
    fn risk_reward_config_sell_stop_above_entry_target_below() {
        let rr = RiskRewardConfig {
            stop_pct: 0.02,
            target_rr: 2.0,
        };
        let (stop, target) = rr.compute(100_000, &Action::Sell);
        assert_eq!(stop, Some(102_000));
        assert_eq!(target, Some(96_000));
    }

    #[test]
    fn risk_reward_config_hold_returns_none_none() {
        let rr = RiskRewardConfig {
            stop_pct: 0.02,
            target_rr: 2.0,
        };
        assert_eq!(rr.compute(100_000, &Action::Hold), (None, None));
    }

    #[test]
    fn risk_reward_config_target_rr_scales_target_distance() {
        let rr = RiskRewardConfig {
            stop_pct: 0.01,
            target_rr: 3.0,
        };
        let (stop, target) = rr.compute(200_000, &Action::Buy);
        // risk = 200_000 * 0.01 = 2_000; stop = 198_000; target = 200_000 + 2_000*3 = 206_000
        assert_eq!(stop, Some(198_000));
        assert_eq!(target, Some(206_000));
    }

    #[test]
    fn risk_reward_config_buy_stop_never_negative_for_large_stop_pct() {
        let rr = RiskRewardConfig {
            stop_pct: 5.0,
            target_rr: 1.0,
        };
        let (stop, _) = rr.compute(100, &Action::Buy);
        assert_eq!(stop, Some(0), "stop must clamp at zero, not underflow");
    }

    #[test]
    fn risk_reward_config_sell_target_never_negative_for_large_stop_pct() {
        let rr = RiskRewardConfig {
            stop_pct: 5.0,
            target_rr: 1.0,
        };
        let (_, target) = rr.compute(100, &Action::Sell);
        assert_eq!(target, Some(0), "target must clamp at zero, not underflow");
    }
}
