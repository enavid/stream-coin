//! Pure formatting helpers for the chart page's trade overlay (backtest
//! closed trades + the stats dashboard table) — split out of `chart.rs`
//! since this file had grown to ~1,800 lines mixing the page component,
//! the JS glue, and these formatters in one place. Each function here
//! mirrors a JS counterpart in `chart/glue.js` (kept in sync by hand, since
//! Series Primitives can't be driven from Rust directly) and exists as a
//! plain Rust function purely so it's unit testable without a Dioxus
//! runtime or a browser.

/// Builds the in-box text drawn on a `TradeRectanglePrimitive` (JS side,
/// `scChartSetTrades`) — kept as a plain Rust function so it's unit
/// testable, rather than living only inside the JS template string.
/// Not yet called from non-test code: the chart page doesn't drive
/// `scChartSetTrades` until it's wired up.
#[allow(dead_code)]
pub(super) fn format_trade_label(trade: &crate::api::ClosedTrade) -> String {
    let side = match trade.side {
        crate::api::TradeSide::Long => "LONG",
        crate::api::TradeSide::Short => "SHORT",
    };
    let mut parts = vec![side.to_string()];
    if let Some(rr) = trade.rr {
        parts.push(format!("RR: {rr:.2}"));
    }
    parts.push(format!("E: {}", trade.entry_price));
    if let Some(sl) = trade.stop_loss {
        parts.push(format!("SL: {sl}"));
    }
    if let Some(tp) = trade.take_profit {
        parts.push(format!("TP: {tp}"));
    }
    parts.join(" | ")
}

/// Which closed trades to actually attach primitives for when the set is
/// large — filters to those whose `entry_time..exit_time` overlaps the
/// currently visible time range, then caps at the `cap` most-recent (by
/// `exit_time`), rather than a hard global drop. Mirrors the recency-within-
/// visible-range approach `ROADMAP.md`'s Phase 7 research found in
/// FreqUI/vectorbt for "too many trades to render legibly".
///
/// When `cursor_time` is `Some`, additionally excludes trades whose
/// `exit_time` is after the cursor (Loop 6i playback): a trade that hasn't
/// been exited yet at the cursor position shouldn't appear as a closed trade
/// in the overlay (the JS layer handles "open position" rendering separately).
///
/// Timestamps are compared as plain strings — RFC3339 with a fixed
/// fractional-second precision and `Z` suffix (this codebase's convention,
/// see `BacktestRunRequest::from`'s doc comment) sorts identically whether
/// compared as text or as parsed instants, so no date library is needed.
#[allow(dead_code)]
pub(super) fn visible_trades<'a>(
    trades: &'a [crate::api::ClosedTrade],
    visible_from: &str,
    visible_to: &str,
    cursor_time: Option<&str>,
    cap: usize,
) -> Vec<&'a crate::api::ClosedTrade> {
    let mut overlapping: Vec<&crate::api::ClosedTrade> = trades
        .iter()
        .filter(|t| t.entry_time.as_str() <= visible_to && t.exit_time.as_str() >= visible_from)
        .filter(|t| {
            cursor_time.is_none_or(|cursor| t.exit_time.as_str() <= cursor)
        })
        .collect();
    overlapping.sort_by(|a, b| b.exit_time.cmp(&a.exit_time));
    overlapping.truncate(cap);
    overlapping
}

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

/// Short code drawn on a closed trade's exit marker (`setMarkers`'
/// `text` field) — independent of `format_trade_label`'s side/RR/E/SL/TP
/// text, which lives in the rectangle instead. Not yet called from
/// non-test code — mirrors the JS `buildTradeMarkers`' exit-marker
/// text until the gap (exit markers + tooltip weren't built in Stage 5)
/// is wired up.
#[allow(dead_code)]
pub(super) fn exit_marker_label(outcome: crate::api::TradeOutcome) -> &'static str {
    match outcome {
        crate::api::TradeOutcome::Win => "W",
        crate::api::TradeOutcome::Loss => "L",
        crate::api::TradeOutcome::Breakeven => "BE",
    }
}

/// Full hover-tooltip text for a closed trade (outcome + PnL%, then the
/// same side/RR/entry/SL/TP line as `format_trade_label`) — kept as a
/// plain Rust function so it's unit testable, same rationale as
/// `format_trade_label`.
#[allow(dead_code)]
pub(super) fn format_trade_tooltip(trade: &crate::api::ClosedTrade) -> String {
    let outcome_text = match trade.outcome {
        crate::api::TradeOutcome::Win => "Win",
        crate::api::TradeOutcome::Loss => "Loss",
        crate::api::TradeOutcome::Breakeven => "Breakeven",
    };
    let sign = if trade.pnl_pct >= 0.0 { "+" } else { "" };
    format!(
        "{outcome_text} {sign}{:.2}% | {}",
        trade.pnl_pct,
        format_trade_label(trade)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_trade(
        side: crate::api::TradeSide,
        rr: Option<f64>,
        stop_loss: Option<u64>,
        take_profit: Option<u64>,
    ) -> crate::api::ClosedTrade {
        crate::api::ClosedTrade {
            strategy_id: "s1".to_string(),
            side,
            entry_price: 100_000,
            exit_price: 110_000,
            stop_loss,
            take_profit,
            quantity: 1,
            entry_time: "2026-01-01T00:00:00Z".to_string(),
            exit_time: "2026-01-01T00:01:00Z".to_string(),
            pnl: 10_000,
            pnl_pct: 10.0,
            rr,
            outcome: crate::api::TradeOutcome::Win,
        }
    }

    #[test]
    fn format_trade_label_includes_side_rr_entry_sl_tp() {
        let trade = closed_trade(
            crate::api::TradeSide::Long,
            Some(2.0),
            Some(95_000),
            Some(110_000),
        );
        let label = format_trade_label(&trade);
        assert!(label.contains("LONG"), "label was: {label}");
        assert!(label.contains("RR: 2.00"), "label was: {label}");
        assert!(label.contains("E: 100000"), "label was: {label}");
        assert!(label.contains("SL: 95000"), "label was: {label}");
        assert!(label.contains("TP: 110000"), "label was: {label}");
    }

    #[test]
    fn format_trade_label_omits_rr_sl_tp_when_absent() {
        let trade = closed_trade(crate::api::TradeSide::Short, None, None, None);
        let label = format_trade_label(&trade);
        assert!(label.contains("SHORT"), "label was: {label}");
        assert!(!label.contains("RR:"), "label was: {label}");
        assert!(!label.contains("SL:"), "label was: {label}");
        assert!(!label.contains("TP:"), "label was: {label}");
    }

    #[test]
    fn exit_marker_label_maps_outcome_to_short_code() {
        assert_eq!(exit_marker_label(crate::api::TradeOutcome::Win), "W");
        assert_eq!(exit_marker_label(crate::api::TradeOutcome::Loss), "L");
        assert_eq!(exit_marker_label(crate::api::TradeOutcome::Breakeven), "BE");
    }

    #[test]
    fn format_trade_tooltip_includes_outcome_and_pnl_pct() {
        let mut trade = closed_trade(crate::api::TradeSide::Long, Some(2.0), None, None);
        trade.outcome = crate::api::TradeOutcome::Win;
        trade.pnl_pct = 9.5;

        let tooltip = format_trade_tooltip(&trade);

        assert!(tooltip.contains("Win"), "tooltip was: {tooltip}");
        assert!(tooltip.contains("+9.50%"), "tooltip was: {tooltip}");
        assert!(tooltip.contains("LONG"), "tooltip was: {tooltip}");
    }

    #[test]
    fn format_trade_tooltip_shows_negative_pnl_pct_without_plus_sign() {
        let mut trade = closed_trade(crate::api::TradeSide::Short, None, None, None);
        trade.outcome = crate::api::TradeOutcome::Loss;
        trade.pnl_pct = -3.2;

        let tooltip = format_trade_tooltip(&trade);

        assert!(tooltip.contains("Loss"), "tooltip was: {tooltip}");
        assert!(tooltip.contains("-3.20%"), "tooltip was: {tooltip}");
        assert!(!tooltip.contains("+-"), "tooltip was: {tooltip}");
    }

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

    #[test]
    fn visible_trades_filters_to_overlapping_range_only() {
        let trades = vec![
            closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z"), // before range
            closed_trade_at("2026-01-01T01:00:00Z", "2026-01-01T01:05:00Z"), // inside range
            closed_trade_at("2026-01-01T03:00:00Z", "2026-01-01T03:05:00Z"), // after range
        ];

        let visible =
            visible_trades(&trades, "2026-01-01T00:30:00Z", "2026-01-01T02:00:00Z", None, 500);

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].entry_time, "2026-01-01T01:00:00Z");
    }

    #[test]
    fn visible_trades_caps_at_500_preferring_most_recent() {
        let trades: Vec<crate::api::ClosedTrade> = (0..600)
            .map(|i| {
                closed_trade_at(
                    &format!("2026-01-01T{:02}:00:00Z", i % 24),
                    &format!("2026-01-02T{:02}:00:00Z", i % 24),
                )
            })
            .collect();

        let visible =
            visible_trades(&trades, "2026-01-01T00:00:00Z", "2026-01-03T00:00:00Z", None, 500);

        assert_eq!(visible.len(), 500);
        assert!(
            visible.windows(2).all(|w| w[0].exit_time >= w[1].exit_time),
            "trades must be ordered most-recent-exit first"
        );
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

    #[test]
    fn trades_visible_at_cursor_excludes_trades_not_yet_exited() {
        let trades = vec![
            // exited before the cursor — should appear
            closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z"),
            // exited after the cursor — should be hidden (not yet closed at cursor)
            closed_trade_at("2026-01-01T00:02:00Z", "2026-01-01T00:05:00Z"),
        ];
        let cursor = "2026-01-01T00:03:00Z";

        let visible = visible_trades(
            &trades,
            "2026-01-01T00:00:00Z",
            "2026-01-01T01:00:00Z",
            Some(cursor),
            500,
        );

        assert_eq!(visible.len(), 1, "only the trade exited before the cursor should be visible");
        assert_eq!(visible[0].exit_time, "2026-01-01T00:01:00Z");
    }

    #[test]
    fn trades_visible_at_cursor_none_disables_cursor_filter() {
        let trades = vec![
            closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z"),
            closed_trade_at("2026-01-01T00:02:00Z", "2026-01-01T00:05:00Z"),
        ];

        let visible = visible_trades(
            &trades,
            "2026-01-01T00:00:00Z",
            "2026-01-01T01:00:00Z",
            None,
            500,
        );

        assert_eq!(visible.len(), 2, "without a cursor both trades must appear");
    }

    #[test]
    fn trades_visible_at_cursor_includes_trade_exited_exactly_at_cursor() {
        let trades = vec![
            // exit_time == cursor_time (boundary: <= includes it)
            closed_trade_at("2026-01-01T00:00:00Z", "2026-01-01T00:03:00Z"),
        ];
        let cursor = "2026-01-01T00:03:00Z";

        let visible = visible_trades(
            &trades,
            "2026-01-01T00:00:00Z",
            "2026-01-01T01:00:00Z",
            Some(cursor),
            500,
        );

        assert_eq!(
            visible.len(),
            1,
            "a trade exited exactly at the cursor must be shown (exit_time <= cursor_time)"
        );
    }
}
