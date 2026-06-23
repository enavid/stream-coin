use dioxus::prelude::*;

use super::current_token;
use crate::api::{ActiveStrategy, ApiClient, DeployStrategyRequest, StartStrategyRequest};
use crate::state::AppState;

const STRATEGY_TYPES: &[&str] = &["spread_threshold", "price_delta"];

#[component]
pub fn Strategies(server_url: String) -> Element {
    let state = use_context::<AppState>();
    // `Signal<ApiClient>` is `Copy`, so every closure below can capture
    // `api` freely and stay `Fn` (reusable across multiple events)
    // without manual re-cloning gymnastics — same pattern Dashboard uses.
    let api = use_signal(|| {
        ApiClient::new(server_url).with_unauthorized_handler(move || {
            let mut state = state;
            state.clear_session();
        })
    });

    let mut active = use_signal(Vec::<ActiveStrategy>::new);
    let mut list_error = use_signal(|| None::<String>);

    let refresh = move || {
        let api = api();
        let token = current_token(&state);
        spawn(async move {
            let Some(token) = token else { return };
            match api.list_strategies(&token).await {
                Ok(list) => active.set(list.strategies),
                Err(e) => list_error.set(Some(e)),
            }
        });
    };

    // Re-runs on mount and whenever the WS transport resyncs after a
    // reconnect (`AppState::resync_epoch`), so a connection drop doesn't
    // leave the strategy list silently stale forever.
    use_effect(move || {
        let _resync = (state.resync_epoch)();
        refresh();
    });

    // --- start built-in form ---
    let mut strategy_id = use_signal(String::new);
    let mut strategy_type = use_signal(|| STRATEGY_TYPES[0].to_string());
    let mut exchange_choice = use_signal(String::new);
    let mut pair_choice = use_signal(String::new);
    let mut params = use_signal(|| "{}".to_string());
    let mut start_error = use_signal(|| None::<String>);
    let mut stop_error = use_signal(|| None::<String>);

    let catalog = state.catalog.read();
    let exchanges = catalog.exchanges().to_vec();
    let selected_exchange = if exchanges.iter().any(|e| e.name == exchange_choice()) {
        exchange_choice()
    } else {
        exchanges
            .first()
            .map(|e| e.name.clone())
            .unwrap_or_default()
    };
    let pairs = catalog.pairs_for(&selected_exchange).to_vec();
    drop(catalog);
    let selected_pair = if pairs
        .iter()
        .any(|p| format!("{}/{}", p.base, p.quote) == pair_choice())
    {
        pair_choice()
    } else {
        pairs
            .first()
            .map(|p| format!("{}/{}", p.base, p.quote))
            .unwrap_or_default()
    };

    // Closures below are `move` and need their own clones — capturing
    // `selected_exchange`/`selected_pair` directly would move them out of
    // this scope, breaking the `{selected_exchange}`/`{selected_pair}`
    // bindings the rsx markup further down still needs to read.
    let exchange_for_start = selected_exchange.clone();
    let pair_for_start = selected_pair.clone();
    let on_start = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let parsed_params = match serde_json::from_str(&params()) {
            Ok(v) => v,
            Err(e) => {
                start_error.set(Some(format!("invalid params JSON: {e}")));
                return;
            }
        };
        let req = StartStrategyRequest {
            strategy_id: strategy_id(),
            strategy_type: strategy_type(),
            exchange: exchange_for_start.clone(),
            pair: pair_for_start.clone(),
            params: parsed_params,
        };
        spawn(async move {
            match api.start_strategy(&token, req).await {
                Ok(()) => refresh(),
                Err(e) => start_error.set(Some(e)),
            }
        });
    };

    let on_stop = move |id: String| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            match api.stop_strategy(&token, &id).await {
                Ok(()) => refresh(),
                Err(e) => stop_error.set(Some(e)),
            }
        });
    };

    // --- deploy python strategy form ---
    let mut py_name = use_signal(String::new);
    let mut py_code = use_signal(|| {
        "from stream_coin import Strategy, Action\n\nclass MyStrategy(Strategy):\n    def on_candle(self, candle):\n        pass\n\nMyStrategy().run()\n".to_string()
    });
    let mut py_params = use_signal(|| "{}".to_string());
    let mut deploy_error = use_signal(|| None::<String>);
    let mut deploy_success = use_signal(|| None::<String>);

    let on_deploy = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let parsed_params = match serde_json::from_str(&py_params()) {
            Ok(v) => v,
            Err(e) => {
                deploy_error.set(Some(format!("invalid params JSON: {e}")));
                return;
            }
        };
        let req = DeployStrategyRequest {
            name: py_name(),
            code: py_code(),
            params: parsed_params,
        };
        spawn(async move {
            match api.deploy_strategy(&token, req).await {
                Ok(deployed) => {
                    deploy_success.set(Some(format!("Deployed as {}", deployed.strategy_id)));
                    deploy_error.set(None);
                    refresh();
                }
                Err(e) => deploy_error.set(Some(e)),
            }
        });
    };

    let signal_rows = state.signals.read().rows().to_vec();

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Strategies" }
                div { class: "page-sub", "Built-in Rust strategies + your own Python code" }
            }
        }

        if let Some(err) = list_error() {
            div { class: "form-error", "{err}" }
        }
        if let Some(err) = stop_error() {
            div { class: "form-error", "{err}" }
        }

        section { class: "block card", style: "padding:0;",
            if active().is_empty() {
                div { style: "padding:16px; color:var(--text-dim); font-size:12.5px;", "No strategies running." }
            }
            for s in active() {
                div { class: "strat-row", key: "{s.strategy_id}",
                    div { class: "strat-main",
                        div {
                            div { class: "strat-name", "{s.strategy_id}" }
                            div { class: "strat-meta", "{s.strategy_type} · {s.exchange} · {s.pair}" }
                        }
                    }
                    div { class: "strat-actions",
                        span { class: "pill pill-green", "● running" }
                        button {
                            class: "btn btn-ghost btn-sm",
                            onclick: {
                                let id = s.strategy_id.clone();
                                move |_| on_stop(id.clone())
                            },
                            "Stop"
                        }
                    }
                }
            }
        }

        section { class: "block card",
            span { class: "label", "Start a built-in strategy" }
            form { onsubmit: on_start,
                div { class: "field-row grid-2", style: "margin-bottom:10px;",
                    div { class: "field",
                        label { "Strategy ID" }
                        input { class: "finput", value: "{strategy_id}", oninput: move |e| strategy_id.set(e.value()) }
                    }
                    div { class: "field",
                        label { "Type" }
                        select {
                            class: "finput",
                            value: "{strategy_type}",
                            onchange: move |e| strategy_type.set(e.value()),
                            for t in STRATEGY_TYPES { option { value: *t, "{t}" } }
                        }
                    }
                }
                div { class: "field-row grid-2", style: "margin-bottom:10px;",
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
                }
                div { class: "field", style: "margin-bottom:14px;",
                    label { "Params (JSON)" }
                    textarea {
                        class: "finput",
                        rows: "3",
                        value: "{params}",
                        oninput: move |e| params.set(e.value()),
                    }
                }
                if let Some(err) = start_error() {
                    div { class: "form-error", "{err}" }
                }
                button { class: "btn btn-primary", r#type: "submit", "Start strategy" }
            }
        }

        section { class: "block",
            span { class: "label", "New Python Strategy" }
            div { class: "card",
                form { onsubmit: on_deploy,
                    div { class: "field", style: "margin-bottom:14px;",
                        label { "Strategy name" }
                        input { class: "finput", value: "{py_name}", oninput: move |e| py_name.set(e.value()) }
                    }
                    div { class: "field", style: "margin-bottom:14px;",
                        div { class: "editor-toolbar",
                            div { class: "dot-row",
                                span { style: "background:#f43f5e" }
                                span { style: "background:#f59e0b" }
                                span { style: "background:#22c55e" }
                            }
                            span { class: "filename", "strategy.py" }
                        }
                        textarea {
                            class: "code-editor",
                            spellcheck: "false",
                            value: "{py_code}",
                            oninput: move |e| py_code.set(e.value()),
                        }
                    }
                    div { class: "field", style: "margin-bottom:14px;",
                        label { "Params (JSON)" }
                        textarea { class: "finput", rows: "2", value: "{py_params}", oninput: move |e| py_params.set(e.value()) }
                    }
                    if let Some(err) = deploy_error() {
                        div { class: "form-error", "{err}" }
                    }
                    if let Some(ok) = deploy_success() {
                        div { class: "form-success", "{ok}" }
                    }
                    button { class: "btn btn-primary", r#type: "submit", "Deploy strategy" }
                }
            }
        }

        section { class: "block",
            span { class: "label", "Live Signals" }
            div { class: "table-wrap",
                table {
                    thead {
                        tr { th { "Time" } th { "Strategy" } th { "Pair" } th { "Action" } th { "Confidence" } }
                    }
                    tbody {
                        for row in signal_rows.iter() {
                            tr { key: "{row.signal_id}",
                                td { class: "mono", "{row.time}" }
                                td { "{row.strategy_id}" }
                                td { class: "mono", "{row.pair}" }
                                td {
                                    span {
                                        class: if row.action == "buy" { "pill pill-green" } else if row.action == "sell" { "pill pill-red" } else { "pill pill-muted" },
                                        "{row.action}"
                                    }
                                }
                                td { class: "mono", "{row.confidence}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
