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
    /// Stop-loss/take-profit the *entry* fill was placed with, if any —
    /// `None` for fills with no `RiskRewardConfig` upstream, and irrelevant
    /// (ignored by `pair_closed_trades`) on the exit fill.
    pub stop_loss: Option<u64>,
    pub take_profit: Option<u64>,
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
    pub closed_trades: Vec<ClosedTrade>,
    pub win_rate: f64,
    pub avg_rr: Option<f64>,
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

/// Pair entry/exit fills into closed trades, per `strategy_id` independently
/// (a single backtest run covers exactly one exchange/pair, so `strategy_id`
/// alone is a sufficient key). The first fill for a strategy opens a
/// position; the next *opposite-side* fill closes it. A same-side fill while
/// already in a position is neither a new entry nor a close — it's ignored
/// for pairing purposes, leaving the original entry in place.
///
/// The entry fill's `stop_loss`/`take_profit` (set when the originating
/// signal carried a `RiskRewardConfig`) are carried into the resulting
/// `ClosedTrade` — the exit fill's own SL/TP, if any, are irrelevant here.
pub fn pair_closed_trades(trade_log: &[TradeRecord]) -> Vec<ClosedTrade> {
    use std::collections::HashMap;

    struct OpenEntry {
        side: TradeSide,
        price: u64,
        quantity: u64,
        time: DateTime<Utc>,
        stop_loss: Option<u64>,
        take_profit: Option<u64>,
    }

    let mut open: HashMap<String, OpenEntry> = HashMap::new();
    let mut closed = Vec::new();

    for record in trade_log {
        let side = match record.side.as_str() {
            "buy" => TradeSide::Long,
            "sell" => TradeSide::Short,
            _ => continue,
        };

        match open.remove(&record.strategy_id) {
            None => {
                open.insert(
                    record.strategy_id.clone(),
                    OpenEntry {
                        side,
                        price: record.fill_price,
                        quantity: record.quantity,
                        time: record.candle_time,
                        stop_loss: record.stop_loss,
                        take_profit: record.take_profit,
                    },
                );
            }
            Some(entry) if entry.side == side => {
                open.insert(record.strategy_id.clone(), entry);
            }
            Some(entry) => {
                closed.push(ClosedTrade::close(
                    record.strategy_id.clone(),
                    entry.side,
                    entry.price,
                    record.fill_price,
                    entry.stop_loss,
                    entry.take_profit,
                    entry.quantity,
                    entry.time,
                    record.candle_time,
                ));
            }
        }
    }

    closed
}

/// Win rate (0.0 when there are no closed trades) and the average
/// risk/reward ratio across closed trades that have one (`None` when none
/// of them do — e.g. because no signal carried a stop-loss).
pub fn trade_stats(closed_trades: &[ClosedTrade]) -> (f64, Option<f64>) {
    if closed_trades.is_empty() {
        return (0.0, None);
    }

    let wins = closed_trades
        .iter()
        .filter(|t| t.outcome == TradeOutcome::Win)
        .count();
    let win_rate = wins as f64 / closed_trades.len() as f64;

    let rrs: Vec<f64> = closed_trades.iter().filter_map(|t| t.rr).collect();
    let avg_rr = if rrs.is_empty() {
        None
    } else {
        Some(rrs.iter().sum::<f64>() / rrs.len() as f64)
    };

    (win_rate, avg_rr)
}

#[cfg(test)]
mod pairing_tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn fill(strategy_id: &str, side: &str, price: u64, time_secs: i64) -> TradeRecord {
        TradeRecord {
            order_id: format!("{strategy_id}-{side}-{time_secs}"),
            side: side.to_string(),
            quantity: 1,
            fill_price: price,
            strategy_id: strategy_id.to_string(),
            candle_time: t(time_secs),
            stop_loss: None,
            take_profit: None,
        }
    }

    fn fill_with_sl_tp(
        strategy_id: &str,
        side: &str,
        price: u64,
        time_secs: i64,
        stop_loss: Option<u64>,
        take_profit: Option<u64>,
    ) -> TradeRecord {
        TradeRecord {
            stop_loss,
            take_profit,
            ..fill(strategy_id, side, price, time_secs)
        }
    }

    #[test]
    fn pairing_opens_trade_on_first_fill_for_pair() {
        let log = vec![fill("s1", "buy", 100_000, 0)];
        let closed = pair_closed_trades(&log);
        assert!(
            closed.is_empty(),
            "a single fill must not produce a closed trade"
        );
    }

    #[test]
    fn pairing_closes_trade_on_opposite_side_fill() {
        let log = vec![
            fill("s1", "buy", 100_000, 0),
            fill("s1", "sell", 110_000, 60),
        ];
        let closed = pair_closed_trades(&log);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].side, TradeSide::Long);
        assert_eq!(closed[0].entry_price, 100_000);
        assert_eq!(closed[0].exit_price, 110_000);
    }

    #[test]
    fn pairing_keeps_position_open_on_same_side_fill() {
        let log = vec![
            fill("s1", "buy", 100_000, 0),
            fill("s1", "buy", 105_000, 60),
            fill("s1", "sell", 110_000, 120),
        ];
        let closed = pair_closed_trades(&log);
        assert_eq!(
            closed.len(),
            1,
            "the duplicate same-side fill must not produce its own closed trade"
        );
        assert_eq!(
            closed[0].entry_price, 100_000,
            "the original entry must be preserved, not overwritten by the duplicate fill"
        );
    }

    #[test]
    fn pairing_handles_multiple_strategies_independently() {
        let log = vec![
            fill("s1", "buy", 100_000, 0),
            fill("s2", "sell", 200_000, 0),
            fill("s1", "sell", 110_000, 60),
            fill("s2", "buy", 190_000, 60),
        ];
        let closed = pair_closed_trades(&log);
        assert_eq!(closed.len(), 2);
        let s1 = closed
            .iter()
            .find(|c| c.strategy_id == "s1")
            .expect("s1 trade must be present");
        let s2 = closed
            .iter()
            .find(|c| c.strategy_id == "s2")
            .expect("s2 trade must be present");
        assert_eq!(s1.side, TradeSide::Long);
        assert_eq!(s2.side, TradeSide::Short);
    }

    #[test]
    fn pairing_ignores_unpaired_trailing_fill() {
        let log = vec![
            fill("s1", "buy", 100_000, 0),
            fill("s1", "buy", 105_000, 60),
        ];
        let closed = pair_closed_trades(&log);
        assert!(
            closed.is_empty(),
            "two same-side fills with no opposite-side close must produce nothing"
        );
    }

    #[test]
    fn pairing_threads_entry_stop_loss_and_take_profit_into_closed_trade() {
        let log = vec![
            fill_with_sl_tp("s1", "buy", 100_000, 0, Some(98_000), Some(104_000)),
            fill("s1", "sell", 110_000, 60),
        ];
        let closed = pair_closed_trades(&log);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].stop_loss, Some(98_000));
        assert_eq!(closed[0].take_profit, Some(104_000));
        assert!(
            closed[0].rr.is_some(),
            "rr must be populated once stop_loss is known"
        );
    }

    #[test]
    fn pairing_ignores_exit_fills_own_stop_loss_take_profit() {
        // The exit fill carries its own (irrelevant) SL/TP — pairing must use
        // only the entry's, not the exit's.
        let log = vec![
            fill_with_sl_tp("s1", "buy", 100_000, 0, Some(98_000), Some(104_000)),
            fill_with_sl_tp("s1", "sell", 110_000, 60, Some(999), Some(999)),
        ];
        let closed = pair_closed_trades(&log);
        assert_eq!(closed[0].stop_loss, Some(98_000));
        assert_eq!(closed[0].take_profit, Some(104_000));
    }

    #[test]
    fn pairing_leaves_sl_tp_none_when_entry_has_none() {
        let log = vec![
            fill("s1", "buy", 100_000, 0),
            fill("s1", "sell", 110_000, 60),
        ];
        let closed = pair_closed_trades(&log);
        assert!(closed[0].stop_loss.is_none());
        assert!(closed[0].take_profit.is_none());
    }
}

#[cfg(test)]
mod trade_stats_tests {
    use super::*;

    fn winning_trade(strategy_id: &str) -> ClosedTrade {
        ClosedTrade::close(
            strategy_id.to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            None,
            None,
            1,
            DateTime::from_timestamp(0, 0).unwrap(),
            DateTime::from_timestamp(60, 0).unwrap(),
        )
    }

    fn losing_trade(strategy_id: &str) -> ClosedTrade {
        ClosedTrade::close(
            strategy_id.to_string(),
            TradeSide::Long,
            100_000,
            90_000,
            None,
            None,
            1,
            DateTime::from_timestamp(0, 0).unwrap(),
            DateTime::from_timestamp(60, 0).unwrap(),
        )
    }

    fn trade_with_rr(strategy_id: &str, stop_loss: u64, rr_expected: f64) -> ClosedTrade {
        let _ = rr_expected;
        ClosedTrade::close(
            strategy_id.to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            Some(stop_loss),
            None,
            1,
            DateTime::from_timestamp(0, 0).unwrap(),
            DateTime::from_timestamp(60, 0).unwrap(),
        )
    }

    #[test]
    fn backtest_result_win_rate_counts_only_closed_trades() {
        let trades = vec![winning_trade("s1"), winning_trade("s1"), losing_trade("s1")];
        let (win_rate, _) = trade_stats(&trades);
        assert!(
            (win_rate - (2.0 / 3.0)).abs() < 1e-9,
            "win rate must be wins / total closed trades, got {win_rate}"
        );
    }

    #[test]
    fn backtest_result_avg_rr_is_none_when_no_trade_has_stop_loss() {
        let trades = vec![winning_trade("s1"), losing_trade("s1")];
        let (_, avg_rr) = trade_stats(&trades);
        assert!(avg_rr.is_none());
    }

    #[test]
    fn backtest_result_avg_rr_averages_only_trades_with_rr() {
        let trades = vec![
            trade_with_rr("s1", 95_000, 2.0), // RR = 10_000/5_000 = 2.0
            winning_trade("s1"),              // no stop_loss -> excluded
        ];
        let (_, avg_rr) = trade_stats(&trades);
        assert_eq!(avg_rr, Some(2.0));
    }

    #[test]
    fn trade_stats_empty_input_returns_zero_win_rate_and_no_avg_rr() {
        let (win_rate, avg_rr) = trade_stats(&[]);
        assert_eq!(win_rate, 0.0);
        assert!(avg_rr.is_none());
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

    #[test]
    fn closed_trade_serializes_with_plain_numeric_fields() {
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
        let value = serde_json::to_value(&trade).unwrap();
        assert!(
            value["entry_price"].is_number(),
            "entry_price must serialize as a plain number, not a string — this module uses the scaled-u64 convention, not Decimal-as-string"
        );
        assert!(value["pnl"].is_number());
        assert_eq!(value["side"], "long");
        assert_eq!(value["outcome"], "win");
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
