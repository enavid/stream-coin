use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub order_id: String,
    pub side: String,
    pub quantity: u64,
    pub fill_price: u64,
    pub strategy_id: String,
    pub candle_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSignalRecord {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub candle_count: usize,
    pub signal_count: usize,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub trade_log: Vec<TradeRecord>,
    pub signal_log: Vec<BacktestSignalRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeSide {
    Long,
    Short,
}

impl TradeSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            TradeSide::Long => "long",
            TradeSide::Short => "short",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeOutcome {
    Win,
    Loss,
    Breakeven,
}

impl TradeOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            TradeOutcome::Win => "win",
            TradeOutcome::Loss => "loss",
            TradeOutcome::Breakeven => "breakeven",
        }
    }
}

/// A fully closed trade — one entry fill paired with its exit fill.
/// Prices and quantity use the same scaled-`u64` convention as `Price`/
/// `TradeRecord`; `pnl` is signed because a loss must be representable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub strategy_id: String,
    pub side: TradeSide,
    pub entry_price: u64,
    pub exit_price: u64,
    pub stop_loss: Option<u64>,
    pub take_profit: Option<u64>,
    pub quantity: u64,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub pnl: i64,
    pub pnl_pct: f64,
    pub rr: Option<f64>,
    pub outcome: TradeOutcome,
}

impl ClosedTrade {
    #[allow(clippy::too_many_arguments)]
    pub fn close(
        strategy_id: String,
        side: TradeSide,
        entry_price: u64,
        exit_price: u64,
        stop_loss: Option<u64>,
        take_profit: Option<u64>,
        quantity: u64,
        entry_time: DateTime<Utc>,
        exit_time: DateTime<Utc>,
    ) -> Self {
        let signed_delta = match side {
            TradeSide::Long => exit_price as i64 - entry_price as i64,
            TradeSide::Short => entry_price as i64 - exit_price as i64,
        };
        let pnl = signed_delta * quantity as i64;

        let cost_basis = entry_price as f64 * quantity as f64;
        let pnl_pct = if cost_basis == 0.0 {
            0.0
        } else {
            pnl as f64 / cost_basis * 100.0
        };

        let rr = stop_loss.and_then(|sl| {
            let risk_per_unit = (entry_price as i64 - sl as i64).unsigned_abs() as f64;
            if risk_per_unit == 0.0 {
                None
            } else {
                Some(signed_delta.unsigned_abs() as f64 / risk_per_unit)
            }
        });

        let outcome = if pnl > 0 {
            TradeOutcome::Win
        } else if pnl < 0 {
            TradeOutcome::Loss
        } else {
            TradeOutcome::Breakeven
        };

        Self {
            strategy_id,
            side,
            entry_price,
            exit_price,
            stop_loss,
            take_profit,
            quantity,
            entry_time,
            exit_time,
            pnl,
            pnl_pct,
            rr,
            outcome,
        }
    }
}

#[cfg(test)]
mod closed_trade_tests {
    use super::*;
    use proptest::prelude::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn closed_trade_pnl_positive_for_winning_long() {
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            None,
            None,
            2,
            t(0),
            t(60),
        );
        assert_eq!(trade.pnl, 20_000);
    }

    #[test]
    fn closed_trade_pnl_negative_for_losing_long() {
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Long,
            100_000,
            90_000,
            None,
            None,
            2,
            t(0),
            t(60),
        );
        assert_eq!(trade.pnl, -20_000);
    }

    #[test]
    fn closed_trade_pnl_positive_for_winning_short() {
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Short,
            100_000,
            90_000,
            None,
            None,
            2,
            t(0),
            t(60),
        );
        assert_eq!(trade.pnl, 20_000);
    }

    #[test]
    fn closed_trade_rr_is_none_when_stop_loss_absent() {
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            None,
            None,
            1,
            t(0),
            t(60),
        );
        assert!(trade.rr.is_none());
    }

    #[test]
    fn closed_trade_rr_computes_distance_ratio_when_stop_loss_present() {
        // Risked 5_000 (entry 100_000, stop 95_000), gained 10_000 -> RR = 2.0
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            Some(95_000),
            None,
            1,
            t(0),
            t(60),
        );
        assert_eq!(trade.rr, Some(2.0));
    }

    #[test]
    fn closed_trade_outcome_breakeven_when_pnl_is_zero() {
        let trade = ClosedTrade::close(
            "s1".to_string(),
            TradeSide::Long,
            100_000,
            100_000,
            None,
            None,
            1,
            t(0),
            t(60),
        );
        assert_eq!(trade.outcome, TradeOutcome::Breakeven);
    }

    proptest! {
        #[test]
        fn closed_trade_pnl_sign_matches_price_direction_long(
            entry in 1_000u64..1_000_000,
            exit in 1_000u64..1_000_000,
            quantity in 1u64..1_000,
        ) {
            let trade = ClosedTrade::close(
                "s1".to_string(),
                TradeSide::Long,
                entry,
                exit,
                None,
                None,
                quantity,
                t(0),
                t(60),
            );
            match exit.cmp(&entry) {
                std::cmp::Ordering::Greater => prop_assert!(trade.pnl > 0),
                std::cmp::Ordering::Less => prop_assert!(trade.pnl < 0),
                std::cmp::Ordering::Equal => prop_assert_eq!(trade.pnl, 0),
            }
        }

        #[test]
        fn closed_trade_pnl_sign_matches_price_direction_short(
            entry in 1_000u64..1_000_000,
            exit in 1_000u64..1_000_000,
            quantity in 1u64..1_000,
        ) {
            let trade = ClosedTrade::close(
                "s1".to_string(),
                TradeSide::Short,
                entry,
                exit,
                None,
                None,
                quantity,
                t(0),
                t(60),
            );
            match exit.cmp(&entry) {
                std::cmp::Ordering::Less => prop_assert!(trade.pnl > 0),
                std::cmp::Ordering::Greater => prop_assert!(trade.pnl < 0),
                std::cmp::Ordering::Equal => prop_assert_eq!(trade.pnl, 0),
            }
        }
    }
}
