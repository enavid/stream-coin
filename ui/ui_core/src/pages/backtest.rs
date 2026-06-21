use dioxus::prelude::*;

use super::current_token;
use crate::api::{ApiClient, BacktestResult, BacktestRunRequest};
use crate::domain::SUPPORTED_EXCHANGES;
use crate::state::AppState;

const INTERVALS: &[&str] = &["1m", "5m", "15m", "1h"];

#[component]
pub fn Backtest(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| ApiClient::new(server_url));

    let mut strategy_id = use_signal(String::new);
    let mut exchange = use_signal(|| SUPPORTED_EXCHANGES[0].to_string());
    let mut pair = use_signal(String::new);
    let mut interval = use_signal(|| INTERVALS[0].to_string());
    let mut from = use_signal(String::new);
    let mut to = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut result = use_signal(|| None::<BacktestResult>);
    let mut running = use_signal(|| false);

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
        let req = BacktestRunRequest {
            strategy_id: strategy_id(),
            exchange: exchange(),
            pair: pair(),
            interval: interval(),
            from: format!("{}T00:00:00Z", from()),
            to: format!("{}T00:00:00Z", to()),
            params: serde_json::json!({}),
        };
        running.set(true);
        spawn(async move {
            let outcome = api.run_backtest(&token, req).await;
            running.set(false);
            match outcome {
                Ok(r) => {
                    result.set(Some(r));
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        });
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
                            value: "{exchange}",
                            onchange: move |e| exchange.set(e.value()),
                            for ex in SUPPORTED_EXCHANGES { option { value: *ex, "{ex}" } }
                        }
                    }
                    div { class: "field",
                        label { "Pair" }
                        input { class: "finput", placeholder: "USDT/IRT", value: "{pair}", oninput: move |e| pair.set(e.value()) }
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
                        input { class: "finput", r#type: "date", value: "{from}", oninput: move |e| from.set(e.value()) }
                    }
                    div { class: "field",
                        label { "To" }
                        input { class: "finput", r#type: "date", value: "{to}", oninput: move |e| to.set(e.value()) }
                    }
                }
                if let Some(err) = error() {
                    div { class: "form-error", "{err}" }
                }
                button {
                    class: "btn btn-primary",
                    r#type: "submit",
                    disabled: running(),
                    if running() { "Running…" } else { "Run backtest" }
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
