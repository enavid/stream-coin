//! Pure formatting helpers for the chart page's trade overlay (the backtest
//! stats dashboard table) — split out of `chart.rs` since this file had grown
//! to ~1,800 lines mixing the page component, the JS glue, and these
//! formatters in one place. `format_stats_row` mirrors a JS counterpart in
//! `chart/glue.js` (kept in sync by hand, since Series Primitives can't be
//! driven from Rust directly) and exists as a plain Rust function purely so
//! it's unit testable without a Dioxus runtime or a browser.

/// CSS class for a sign-colored stats cell — green/red/neutral by plain
/// class toggle (per `ROADMAP.md` Phase 7: "plain CSS class toggle, not
/// inline-computed colors").
fn sign_class(value: f64) -> &'static str {
    if value > 0.0 {
        "stat-positive"
    } else if value < 0.0 {
        "stat-negative"
    } else {
        "stat-neutral"
    }
}

/// The 2-column x 5-row corner stats table's content for one backtest
/// result — title | total trades | win rate | total PnL | avg RR.
#[derive(serde::Serialize)]
pub(super) struct StatsRow {
    pub title: String,
    pub total_trades: usize,
    pub win_rate_pct: String,
    pub win_rate_class: &'static str,
    pub total_pnl: i64,
    pub total_pnl_class: &'static str,
    pub avg_rr: String,
}

/// Builds the stats row from a `BacktestResult` — kept as a plain Rust
/// function so the win-rate/PnL sign coloring is unit testable, rather than
/// computed only inside the JS DOM-overlay template (`scChartSetStats`).
pub(super) fn format_stats_row(result: &crate::api::BacktestResult) -> StatsRow {
    let total_pnl: i64 = result.closed_trades.iter().map(|t| t.pnl).sum();
    StatsRow {
        title: format!("{} · {}", result.strategy_id, result.pair),
        total_trades: result.closed_trades.len(),
        win_rate_pct: format!("{:.1}%", result.win_rate * 100.0),
        win_rate_class: sign_class(result.win_rate - 0.5),
        total_pnl,
        total_pnl_class: sign_class(total_pnl as f64),
        avg_rr: result
            .avg_rr
            .map(|rr| format!("{rr:.2}"))
            .unwrap_or_else(|| "—".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_trade_at(entry_time: &str, exit_time: &str) -> crate::api::ClosedTrade {
        crate::api::ClosedTrade {
            strategy_id: "s1".to_string(),
            side: crate::api::TradeSide::Long,
            entry_price: 100_000,
            exit_price: 110_000,
            stop_loss: None,
            take_profit: None,
            quantity: 1,
            entry_time: entry_time.to_string(),
            exit_time: exit_time.to_string(),
            pnl: 10_000,
            pnl_pct: 10.0,
            rr: None,
            outcome: crate::api::TradeOutcome::Win,
        }
    }

    fn backtest_result_with_trades(
        trades: Vec<crate::api::ClosedTrade>,
    ) -> crate::api::BacktestResult {
        crate::api::BacktestResult {
            strategy_id: "s1".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            candle_count: 10,
            signal_count: 1,
            total_return_pct: 0.0,
            max_drawdown_pct: 0.0,
            trade_log: vec![],
            signal_log: vec![],
            win_rate: 0.0,
            avg_rr: None,
            closed_trades: trades,
        }
    }

    #[test]
    fn format_stats_row_colors_negative_pnl_class() {
        let mut trade = closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z");
        trade.pnl = -5_000;
        let result = backtest_result_with_trades(vec![trade]);

        let row = format_stats_row(&result);

        assert_eq!(row.total_pnl, -5_000);
        assert_eq!(row.total_pnl_class, "stat-negative");
    }

    #[test]
    fn format_stats_row_colors_positive_win_rate_class() {
        let mut result = backtest_result_with_trades(vec![]);
        result.win_rate = 0.75;

        let row = format_stats_row(&result);

        assert_eq!(row.win_rate_class, "stat-positive");
    }

    #[test]
    fn format_stats_row_sums_pnl_across_closed_trades() {
        let mut a = closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z");
        a.pnl = 10_000;
        let mut b = closed_trade_at("2026-01-01T01:00:00Z", "2026-01-01T01:01:00Z");
        b.pnl = -3_000;
        let result = backtest_result_with_trades(vec![a, b]);

        let row = format_stats_row(&result);

        assert_eq!(row.total_trades, 2);
        assert_eq!(row.total_pnl, 7_000);
        assert_eq!(row.total_pnl_class, "stat-positive");
    }
}
