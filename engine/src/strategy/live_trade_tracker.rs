use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::backtest::entity::{ClosedTrade, TradeSide};
use crate::strategy::entity::{Action, Signal};

/// Fixed quantity used for every live-preview position — mirrors the
/// backtest engine's `DEFAULT_ORDER_QUANTITY`, since neither path has a real
/// quantity to work with (`Signal`/`SignalPayload` carry none) and the two
/// must agree for `pnl`/`pnl_pct` to be directly comparable.
const LIVE_PREVIEW_QUANTITY: u64 = 1;

struct OpenPosition {
    side: TradeSide,
    entry_price: u64,
    quantity: u64,
    stop_loss: Option<u64>,
    take_profit: Option<u64>,
    entry_time: DateTime<Utc>,
}

/// Watches a strategy's live signals and turns entry/exit pairs into
/// `ClosedTrade`s, without a historical replay engine — "watching a
/// strategy already running live," per Loop 6h. Mirrors
/// `backtest::entity::pair_closed_trades`'s state machine exactly, except
/// incrementally (one signal/candle at a time) instead of post-hoc over a
/// full trade log.
///
/// Keyed by `strategy_id` so one tracker instance could in principle serve
/// several strategies, though today's only caller (`spawn_strategy_runner`)
/// creates one tracker per single running strategy.
#[derive(Default)]
pub struct LiveTradeTracker {
    open: Mutex<HashMap<String, OpenPosition>>,
}

impl LiveTradeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one signal at `price` (the reference price the strategy used to
    /// compute the signal — `Price::ask` or `Candle::close`, same value the
    /// emitting strategy's `RiskRewardConfig::compute` was given). Returns a
    /// `ClosedTrade` exactly when this signal closes an existing opposite-side
    /// position; `Hold`, a same-side signal, or a signal that opens a new
    /// position all return `None`.
    pub fn on_signal(
        &self,
        signal: &Signal,
        price: u64,
        time: DateTime<Utc>,
    ) -> Option<ClosedTrade> {
        let side = match signal.action {
            Action::Buy => TradeSide::Long,
            Action::Sell => TradeSide::Short,
            Action::Hold => return None,
        };

        let mut open = self.open.lock().unwrap();
        match open.remove(&signal.strategy_id) {
            None => {
                open.insert(
                    signal.strategy_id.clone(),
                    OpenPosition {
                        side,
                        entry_price: price,
                        quantity: LIVE_PREVIEW_QUANTITY,
                        stop_loss: signal.stop_loss,
                        take_profit: signal.take_profit,
                        entry_time: time,
                    },
                );
                tracing::debug!(
                    strategy_id = %signal.strategy_id,
                    side = side.as_str(),
                    entry_price = price,
                    "live trade tracker opened position"
                );
                None
            }
            Some(existing) if existing.side == side => {
                // Same-side signal while already in a position — not a new
                // entry, leave the original entry untouched.
                open.insert(signal.strategy_id.clone(), existing);
                None
            }
            Some(entry) => {
                let closed = ClosedTrade::close(
                    signal.strategy_id.clone(),
                    entry.side,
                    entry.entry_price,
                    price,
                    entry.stop_loss,
                    entry.take_profit,
                    entry.quantity,
                    entry.entry_time,
                    time,
                );
                tracing::info!(
                    strategy_id = %signal.strategy_id,
                    side = entry.side.as_str(),
                    entry_price = entry.entry_price,
                    exit_price = price,
                    pnl = closed.pnl,
                    pnl_pct = closed.pnl_pct,
                    outcome = closed.outcome.as_str(),
                    "live trade tracker closed position on opposite signal"
                );
                Some(closed)
            }
        }
    }

    /// Checks `strategy_id`'s open position (if any) against a candle's
    /// `[low, high]` range, forcing an exit at the stop-loss or take-profit
    /// price when touched — same intrabar logic and conservative tie-break
    /// (stop-loss wins) as `SimulatedVenue::apply_candle_close` (Loop 6f),
    /// applied to the live feed instead of historical replay.
    pub fn check_intrabar_stop_loss_take_profit(
        &self,
        strategy_id: &str,
        low: u64,
        high: u64,
        time: DateTime<Utc>,
    ) -> Option<ClosedTrade> {
        let mut open = self.open.lock().unwrap();
        let entry = open.get(strategy_id)?;

        let (sl_hit, tp_hit) = match entry.side {
            TradeSide::Long => (
                entry.stop_loss.is_some_and(|sl| low <= sl),
                entry.take_profit.is_some_and(|tp| high >= tp),
            ),
            TradeSide::Short => (
                entry.stop_loss.is_some_and(|sl| high >= sl),
                entry.take_profit.is_some_and(|tp| low <= tp),
            ),
        };

        let exit_price = if sl_hit {
            entry.stop_loss
        } else if tp_hit {
            entry.take_profit
        } else {
            None
        }?;

        let entry = open.remove(strategy_id)?;
        let closed = ClosedTrade::close(
            strategy_id.to_string(),
            entry.side,
            entry.entry_price,
            exit_price,
            entry.stop_loss,
            entry.take_profit,
            entry.quantity,
            entry.entry_time,
            time,
        );
        tracing::info!(
            strategy_id = strategy_id,
            side = entry.side.as_str(),
            exit_price,
            sl_hit,
            tp_hit,
            pnl = closed.pnl,
            outcome = closed.outcome.as_str(),
            "live trade tracker forced sl/tp exit"
        );
        Some(closed)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use proptest::prelude::*;

    use super::*;
    use crate::backtest::entity::{pair_closed_trades, TradeRecord};

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn buy_signal(strategy_id: &str) -> Signal {
        Signal {
            strategy_id: strategy_id.to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: Action::Buy,
            confidence: 1.0,
            timestamp: t(0),
            stop_loss: None,
            take_profit: None,
        }
    }

    fn sell_signal(strategy_id: &str) -> Signal {
        Signal {
            action: Action::Sell,
            ..buy_signal(strategy_id)
        }
    }

    #[test]
    fn live_trade_tracker_opens_position_on_first_signal() {
        let tracker = LiveTradeTracker::new();
        let result = tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        assert!(
            result.is_none(),
            "opening a position must not emit a ClosedTrade"
        );
    }

    #[test]
    fn live_trade_tracker_closes_and_emits_on_opposite_signal() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        let closed = tracker
            .on_signal(&sell_signal("s1"), 110_000, t(60))
            .expect("opposite-side signal must close the position");

        assert_eq!(closed.side, TradeSide::Long);
        assert_eq!(closed.entry_price, 100_000);
        assert_eq!(closed.exit_price, 110_000);
        assert!(closed.pnl > 0, "buy low sell high must be profitable");
    }

    #[test]
    fn live_trade_tracker_ignores_same_side_signal_keeping_original_entry() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        let result = tracker.on_signal(&buy_signal("s1"), 105_000, t(30));
        assert!(
            result.is_none(),
            "duplicate same-side signal must not close anything"
        );

        let closed = tracker
            .on_signal(&sell_signal("s1"), 110_000, t(60))
            .unwrap();
        assert_eq!(
            closed.entry_price, 100_000,
            "the original entry must be preserved, not overwritten by the duplicate"
        );
    }

    #[test]
    fn live_trade_tracker_ignores_hold_action() {
        let tracker = LiveTradeTracker::new();
        let hold = Signal {
            action: Action::Hold,
            ..buy_signal("s1")
        };
        assert!(tracker.on_signal(&hold, 100_000, t(0)).is_none());
    }

    #[test]
    fn live_trade_tracker_handles_multiple_strategies_independently() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        tracker.on_signal(&sell_signal("s2"), 200_000, t(0));

        let s1_closed = tracker
            .on_signal(&sell_signal("s1"), 110_000, t(60))
            .unwrap();
        let s2_closed = tracker
            .on_signal(&buy_signal("s2"), 190_000, t(60))
            .unwrap();

        assert_eq!(s1_closed.side, TradeSide::Long);
        assert_eq!(s2_closed.side, TradeSide::Short);
    }

    #[test]
    fn live_trade_tracker_reopens_after_closing() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        tracker.on_signal(&sell_signal("s1"), 110_000, t(60));

        // After closing, a fresh buy must open a brand-new position, not
        // immediately "close" anything.
        let result = tracker.on_signal(&buy_signal("s1"), 120_000, t(120));
        assert!(result.is_none());
    }

    #[test]
    fn live_trade_tracker_unpaired_signal_emits_nothing() {
        let tracker = LiveTradeTracker::new();
        let result = tracker.on_signal(&buy_signal("s1"), 100_000, t(0));
        assert!(
            result.is_none(),
            "a single signal with no close must never emit"
        );
    }

    // --- check_intrabar_stop_loss_take_profit ---

    fn buy_with_sl_tp(strategy_id: &str, stop_loss: u64, take_profit: u64) -> Signal {
        Signal {
            stop_loss: Some(stop_loss),
            take_profit: Some(take_profit),
            ..buy_signal(strategy_id)
        }
    }

    #[test]
    fn live_trade_tracker_intrabar_closes_on_stop_loss_touch() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_with_sl_tp("s1", 95_000, 110_000), 100_000, t(0));

        let closed = tracker
            .check_intrabar_stop_loss_take_profit("s1", 94_000, 101_000, t(60))
            .expect("low crossing stop_loss must force a close");
        assert_eq!(closed.exit_price, 95_000);
    }

    #[test]
    fn live_trade_tracker_intrabar_closes_on_take_profit_touch() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_with_sl_tp("s1", 95_000, 110_000), 100_000, t(0));

        let closed = tracker
            .check_intrabar_stop_loss_take_profit("s1", 99_000, 112_000, t(60))
            .expect("high crossing take_profit must force a close");
        assert_eq!(closed.exit_price, 110_000);
    }

    #[test]
    fn live_trade_tracker_intrabar_prefers_stop_loss_on_same_candle_conflict() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_with_sl_tp("s1", 95_000, 110_000), 100_000, t(0));

        let closed = tracker
            .check_intrabar_stop_loss_take_profit("s1", 90_000, 120_000, t(60))
            .unwrap();
        assert_eq!(
            closed.exit_price, 95_000,
            "stop-loss must win when both levels are touched"
        );
    }

    #[test]
    fn live_trade_tracker_intrabar_no_open_position_returns_none() {
        let tracker = LiveTradeTracker::new();
        assert!(tracker
            .check_intrabar_stop_loss_take_profit("nonexistent", 1, 1_000_000, t(0))
            .is_none());
    }

    #[test]
    fn live_trade_tracker_intrabar_range_misses_both_levels_returns_none() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_with_sl_tp("s1", 95_000, 110_000), 100_000, t(0));
        assert!(tracker
            .check_intrabar_stop_loss_take_profit("s1", 99_000, 101_000, t(60))
            .is_none());
    }

    #[test]
    fn live_trade_tracker_intrabar_does_not_double_close() {
        let tracker = LiveTradeTracker::new();
        tracker.on_signal(&buy_with_sl_tp("s1", 95_000, 110_000), 100_000, t(0));
        let first = tracker.check_intrabar_stop_loss_take_profit("s1", 90_000, 96_000, t(60));
        assert!(first.is_some());
        let second = tracker.check_intrabar_stop_loss_take_profit("s1", 90_000, 96_000, t(120));
        assert!(
            second.is_none(),
            "an already-closed position must not close twice"
        );
    }

    #[test]
    fn live_trade_tracker_intrabar_short_position_stop_on_high() {
        let tracker = LiveTradeTracker::new();
        let sell = Signal {
            stop_loss: Some(105_000),
            take_profit: Some(90_000),
            ..sell_signal("s1")
        };
        tracker.on_signal(&sell, 100_000, t(0));

        let closed = tracker
            .check_intrabar_stop_loss_take_profit("s1", 99_000, 106_000, t(60))
            .expect("high crossing a short's stop must force a close");
        assert_eq!(closed.exit_price, 105_000);
    }

    // --- equivalence with the backtest's post-hoc pairing ---

    proptest! {
        #[test]
        fn live_trade_tracker_computes_pnl_pct_same_as_backtest_pair_closed_trades(
            entry_price in 1u64..10_000_000,
            exit_price in 1u64..10_000_000,
            is_long in proptest::bool::ANY,
        ) {
            let (entry_side_str, exit_side_str) = if is_long { ("buy", "sell") } else { ("sell", "buy") };

            // Backtest path: post-hoc pairing over a trade log.
            let trade_log = vec![
                TradeRecord {
                    order_id: "entry".to_string(),
                    side: entry_side_str.to_string(),
                    quantity: 1,
                    fill_price: entry_price,
                    strategy_id: "s1".to_string(),
                    candle_time: t(0),
                    stop_loss: None,
                    take_profit: None,
                },
                TradeRecord {
                    order_id: "exit".to_string(),
                    side: exit_side_str.to_string(),
                    quantity: 1,
                    fill_price: exit_price,
                    strategy_id: "s1".to_string(),
                    candle_time: t(60),
                    stop_loss: None,
                    take_profit: None,
                },
            ];
            let backtest_closed = pair_closed_trades(&trade_log);
            prop_assert_eq!(backtest_closed.len(), 1);

            // Live path: the same two signals, fed incrementally.
            let tracker = LiveTradeTracker::new();
            let action = if is_long { Action::Buy } else { Action::Sell };
            let opposite_action = if is_long { Action::Sell } else { Action::Buy };
            let entry_signal = Signal { action, ..buy_signal("s1") };
            let exit_signal = Signal { action: opposite_action, ..buy_signal("s1") };

            tracker.on_signal(&entry_signal, entry_price, t(0));
            let live_closed = tracker
                .on_signal(&exit_signal, exit_price, t(60))
                .expect("opposite signal must close the live position");

            prop_assert_eq!(live_closed.pnl, backtest_closed[0].pnl);
            prop_assert!((live_closed.pnl_pct - backtest_closed[0].pnl_pct).abs() < 1e-9);
        }
    }
}
