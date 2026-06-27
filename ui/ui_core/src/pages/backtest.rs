use dioxus::prelude::*;

use super::current_token;
use crate::api::{
    candle_count_is_below_expected, expected_candle_count, ApiClient, BackfillRequest,
    BacktestResult, BacktestRunRequest, StartStrategyRequest,
};
use crate::state::AppState;

const INTERVALS: &[&str] = &["1m", "5m", "15m", "1h"];
const STRATEGY_TYPES: &[&str] = &["spread_threshold", "price_delta"];

/// How many recent candles to fetch when sampling whether the selected
/// date range has gaps — generous enough that a `1m` range over a few
/// days still won't be truncated by the sample itself before the real
/// comparison against [`expected_candle_count`] runs.
const GAP_CHECK_LIMIT: u32 = 5000;

/// What `Backtest`'s submit button should do — swapped by the "Watch
/// live" toggle. Loop 6h's `LiveTradeTracker` makes a running strategy's
/// trade closes arrive as `WsMessage::ClosedTrade`, which lands in the
/// same `BacktestStore`/chart overlay a historical `BacktestResult` does,
/// so the two paths only differ in which REST call kicks them off.
#[derive(Debug, Clone, PartialEq)]
enum BacktestAction {
    RunHistorical(BacktestRunRequest),
    StartLive(StartStrategyRequest),
}

/// The form's current field values, gathered into one struct purely to
/// keep [`build_backtest_action`] under clippy's argument-count limit —
/// not a long-lived domain type.
struct BacktestFormFields<'a> {
    strategy_id: &'a str,
    strategy_type: &'a str,
    exchange: &'a str,
    pair: &'a str,
    interval: &'a str,
    from: &'a str,
    to: &'a str,
}

/// Pure decision of which REST call the submit button should fire —
/// extracted so the swap can be unit tested without a Dioxus runtime.
fn build_backtest_action(watch_live: bool, fields: &BacktestFormFields) -> BacktestAction {
    if watch_live {
        BacktestAction::StartLive(StartStrategyRequest {
            strategy_id: fields.strategy_id.to_string(),
            strategy_type: fields.strategy_type.to_string(),
            exchange: fields.exchange.to_string(),
            pair: fields.pair.to_string(),
            params: serde_json::json!({}),
        })
    } else {
        BacktestAction::RunHistorical(BacktestRunRequest {
            strategy_id: fields.strategy_id.to_string(),
            exchange: fields.exchange.to_string(),
            pair: fields.pair.to_string(),
            interval: fields.interval.to_string(),
            from: format!("{}T00:00:00Z", fields.from),
            to: format!("{}T00:00:00Z", fields.to),
            params: serde_json::json!({}),
        })
    }
}

/// Whether the date-range picker should prompt for a backfill before
/// allowing a historical run — `None` (an interval the gap estimate
/// doesn't recognize) never prompts, since there's nothing to compare
/// against.
fn should_prompt_backfill(interval: &str, from: &str, to: &str, actual_count: usize) -> bool {
    match expected_candle_count(interval, from, to) {
        Some(expected) => candle_count_is_below_expected(actual_count, expected),
        None => false,
    }
}

#[component]
pub fn Backtest(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| {
        ApiClient::new(server_url).with_unauthorized_handler(move || {
            let mut state = state;
            state.clear_session();
        })
    });

    let mut strategy_id = use_signal(String::new);
    let mut strategy_type = use_signal(|| STRATEGY_TYPES[0].to_string());
    let mut exchange_choice = use_signal(String::new);
    let mut pair_choice = use_signal(String::new);
    let mut interval = use_signal(|| INTERVALS[0].to_string());
    let mut from = use_signal(String::new);
    let mut to = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut result = use_signal(|| None::<BacktestResult>);
    let mut running = use_signal(|| false);
    let mut watch_live = use_signal(|| false);
    let mut live_active = use_signal(|| false);
    let mut pending_request = use_signal(|| None::<BacktestRunRequest>);
    let mut backfilling = use_signal(|| false);

    let catalog = state.catalog.read();
    let exchanges = catalog.exchanges().to_vec();
    let selected_exchange = catalog.resolve_exchange(&exchange_choice());
    let pairs = catalog.pairs_for(&selected_exchange).to_vec();
    let selected_pair = catalog.resolve_pair(&selected_exchange, &pair_choice());
    drop(catalog);

    let exchange_for_submit = selected_exchange.clone();
    let pair_for_submit = selected_pair.clone();

    let mut run_now = move |req: BacktestRunRequest| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        running.set(true);
        let mut state = state;
        spawn(async move {
            let outcome = api.run_backtest(&token, req).await;
            running.set(false);
            match outcome {
                Ok(r) => {
                    state.backtest.write().set(r.clone());
                    result.set(Some(r));
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        if from().is_empty() || to().is_empty() {
            error.set(Some("'from' and 'to' are required".to_string()));
            return;
        }
        let strategy_id_value = strategy_id();
        let strategy_type_value = strategy_type();
        let interval_value = interval();
        let from_value = from();
        let to_value = to();
        let action = build_backtest_action(
            watch_live(),
            &BacktestFormFields {
                strategy_id: &strategy_id_value,
                strategy_type: &strategy_type_value,
                exchange: &exchange_for_submit,
                pair: &pair_for_submit,
                interval: &interval_value,
                from: &from_value,
                to: &to_value,
            },
        );
        match action {
            BacktestAction::StartLive(req) => {
                let mut state = state;
                state.backtest.write().clear();
                result.set(None);
                running.set(true);
                spawn(async move {
                    let outcome = api.start_strategy(&token, req).await;
                    running.set(false);
                    match outcome {
                        Ok(()) => {
                            live_active.set(true);
                            error.set(None);
                        }
                        Err(e) => error.set(Some(e)),
                    }
                });
            }
            BacktestAction::RunHistorical(req) => {
                let exchange = req.exchange.clone();
                let pair = req.pair.clone();
                let interval_str = req.interval.clone();
                let from_str = from();
                let to_str = to();
                running.set(true);
                spawn(async move {
                    let gap_check = api
                        .list_candles(&token, &exchange, &pair, &interval_str, GAP_CHECK_LIMIT)
                        .await;
                    let needs_backfill = match &gap_check {
                        Ok(items) => {
                            should_prompt_backfill(&interval_str, &from_str, &to_str, items.len())
                        }
                        Err(_) => false,
                    };
                    running.set(false);
                    if needs_backfill {
                        pending_request.set(Some(req));
                    } else {
                        pending_request.set(None);
                        run_now(req);
                    }
                });
            }
        }
    };

    let stop_live = {
        let strategy_id_for_stop = strategy_id();
        move |_| {
            let api = api();
            let Some(token) = current_token(&state) else {
                return;
            };
            let strategy_id = strategy_id_for_stop.clone();
            spawn(async move {
                let _ = api.stop_strategy(&token, &strategy_id).await;
                live_active.set(false);
            });
        }
    };

    let backfill_then_run = move |_| {
        let Some(req) = pending_request() else {
            return;
        };
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let backfill_req = BackfillRequest {
            exchange: req.exchange.clone(),
            pair: req.pair.clone(),
            interval: req.interval.clone(),
            from: req.from.clone(),
            to: req.to.clone(),
        };
        backfilling.set(true);
        spawn(async move {
            let outcome = api.backfill_candles(&token, backfill_req).await;
            backfilling.set(false);
            match outcome {
                Ok(_) => {
                    pending_request.set(None);
                    run_now(req);
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };

    let run_anyway = move |_| {
        if let Some(req) = pending_request() {
            pending_request.set(None);
            run_now(req);
        }
    };

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Backtest" }
                div { class: "page-sub", "Replay historical candles through the same strategy code" }
            }
        }

        section { class: "block card",
            form { onsubmit: on_submit,
                div { class: "field-row grid-4", style: "margin-bottom:12px;",
                    div { class: "field",
                        label { "Strategy ID" }
                        input { class: "finput", value: "{strategy_id}", oninput: move |e| strategy_id.set(e.value()) }
                    }
                    div { class: "field",
                        label { "Exchange" }
                        select {
                            class: "finput",
                            value: "{selected_exchange}",
                            onchange: move |e| {
                                exchange_choice.set(e.value());
                                pair_choice.set(String::new());
                            },
                            for ex in exchanges.iter() { option { value: "{ex.name}", "{ex.name}" } }
                        }
                    }
                    div { class: "field",
                        label { "Pair" }
                        select {
                            class: "finput",
                            value: "{selected_pair}",
                            onchange: move |e| pair_choice.set(e.value()),
                            for p in pairs.iter() { option { value: "{p.base}/{p.quote}", "{p.base}/{p.quote}" } }
                        }
                    }
                    div { class: "field",
                        label { "Interval" }
                        select {
                            class: "finput",
                            value: "{interval}",
                            onchange: move |e| interval.set(e.value()),
                            for i in INTERVALS { option { value: *i, "{i}" } }
                        }
                    }
                }
                div { class: "field-row grid-2", style: "margin-bottom:14px;",
                    div { class: "field",
                        label { "From" }
                        input {
                            class: "finput",
                            r#type: "date",
                            value: "{from}",
                            disabled: watch_live(),
                            oninput: move |e| from.set(e.value()),
                        }
                    }
                    div { class: "field",
                        label { "To" }
                        input {
                            class: "finput",
                            r#type: "date",
                            value: "{to}",
                            disabled: watch_live(),
                            oninput: move |e| to.set(e.value()),
                        }
                    }
                }
                div { class: "field-row grid-2", style: "margin-bottom:14px; align-items:center;",
                    label { class: "field-checkbox",
                        input {
                            r#type: "checkbox",
                            checked: watch_live(),
                            disabled: live_active(),
                            onchange: move |e| watch_live.set(e.checked()),
                        }
                        " Watch live — start the strategy and stream its trade closes instead of replaying history"
                    }
                    if watch_live() {
                        div { class: "field",
                            label { "Strategy type" }
                            select {
                                class: "finput",
                                value: "{strategy_type}",
                                onchange: move |e| strategy_type.set(e.value()),
                                for t in STRATEGY_TYPES { option { value: *t, "{t}" } }
                            }
                        }
                    }
                }
                if let Some(err) = error() {
                    div { class: "form-error", "{err}" }
                }
                if pending_request().is_some() {
                    div { class: "form-warning",
                        "This range looks like it has gaps in the stored candles — backfill before running, or run anyway against whatever history exists."
                        div { class: "field-row", style: "margin-top:8px;",
                            button {
                                class: "btn btn-secondary",
                                r#type: "button",
                                disabled: backfilling(),
                                onclick: backfill_then_run,
                                if backfilling() { "Backfilling…" } else { "Backfill then run" }
                            }
                            button {
                                class: "btn",
                                r#type: "button",
                                disabled: backfilling(),
                                onclick: run_anyway,
                                "Run anyway"
                            }
                        }
                    }
                }
                if live_active() {
                    div { class: "field-row", style: "margin-top:8px;",
                        button {
                            class: "btn btn-secondary",
                            r#type: "button",
                            onclick: stop_live,
                            "Stop watching"
                        }
                    }
                } else {
                    button {
                        class: "btn btn-primary",
                        r#type: "submit",
                        disabled: running(),
                        if running() {
                            "Running…"
                        } else if watch_live() {
                            "Start watching"
                        } else {
                            "Run backtest"
                        }
                    }
                }
            }
        }

        if let Some(r) = result() {
            section { class: "block",
                span { class: "label", "Results" }
                div { class: "stat-row", style: "margin-bottom:14px;",
                    div { class: "stat-card",
                        div { class: "stat-label", "Total Return" }
                        div {
                            class: if r.total_return_pct >= 0.0 { "stat-value stat-pos" } else { "stat-value stat-neg" },
                            "{r.total_return_pct:.2}%"
                        }
                    }
                    div { class: "stat-card",
                        div { class: "stat-label", "Max Drawdown" }
                        div { class: "stat-value stat-neg", "{r.max_drawdown_pct:.2}%" }
                    }
                    div { class: "stat-card",
                        div { class: "stat-label", "Trades" }
                        div { class: "stat-value", "{r.trade_log.len()}" }
                    }
                    div { class: "stat-card",
                        div { class: "stat-label", "Signals" }
                        div { class: "stat-value", "{r.signal_count}" }
                    }
                }
                div { class: "table-wrap",
                    table {
                        thead {
                            tr { th { "Time" } th { "Side" } th { "Qty" } th { "Fill Price" } th { "Order" } }
                        }
                        tbody {
                            for (i, t) in r.trade_log.iter().enumerate() {
                                tr { key: "{i}",
                                    td { class: "mono", "{t.candle_time}" }
                                    td {
                                        span {
                                            class: if t.side == "buy" { "pill pill-green" } else { "pill pill-red" },
                                            "{t.side}"
                                        }
                                    }
                                    td { class: "mono", "{t.quantity}" }
                                    td { class: "mono", "{t.fill_price}" }
                                    td { class: "mono", "{t.order_id}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(strategy_type: &'a str) -> BacktestFormFields<'a> {
        BacktestFormFields {
            strategy_id: "s1",
            strategy_type,
            exchange: "tabdeal",
            pair: "USDT/IRT",
            interval: "1h",
            from: "2026-05-01",
            to: "2026-05-02",
        }
    }

    #[test]
    fn watch_live_toggle_swaps_backtest_run_for_strategy_start() {
        let historical = build_backtest_action(false, &fields("spread_threshold"));
        assert_eq!(
            historical,
            BacktestAction::RunHistorical(BacktestRunRequest {
                strategy_id: "s1".to_string(),
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1h".to_string(),
                from: "2026-05-01T00:00:00Z".to_string(),
                to: "2026-05-02T00:00:00Z".to_string(),
                params: serde_json::json!({}),
            })
        );

        let live = build_backtest_action(true, &fields("spread_threshold"));
        assert_eq!(
            live,
            BacktestAction::StartLive(StartStrategyRequest {
                strategy_id: "s1".to_string(),
                strategy_type: "spread_threshold".to_string(),
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                params: serde_json::json!({}),
            })
        );
    }

    #[test]
    fn should_prompt_backfill_true_when_candle_count_is_below_expected_for_range() {
        // 1h interval over one day expects 24 candles; only 5 are present.
        assert!(should_prompt_backfill(
            "1h",
            "2026-05-01T00:00:00Z",
            "2026-05-02T00:00:00Z",
            5
        ));
    }

    #[test]
    fn should_prompt_backfill_false_when_candle_count_meets_expected_for_range() {
        assert!(!should_prompt_backfill(
            "1h",
            "2026-05-01T00:00:00Z",
            "2026-05-02T00:00:00Z",
            24
        ));
    }

    #[test]
    fn should_prompt_backfill_false_for_an_interval_without_a_gap_estimate() {
        assert!(!should_prompt_backfill(
            "3m",
            "2026-05-01T00:00:00Z",
            "2026-05-02T00:00:00Z",
            0
        ));
    }
}
