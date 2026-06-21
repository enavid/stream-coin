use dioxus::prelude::*;

use super::current_token;
use crate::api::{ApiClient, PlaceOrderRequest};
use crate::domain::SUPPORTED_EXCHANGES;
use crate::state::AppState;

const SIDES: &[&str] = &["buy", "sell"];
const ORDER_TYPES: &[&str] = &["market", "limit"];

#[component]
pub fn Orders(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| ApiClient::new(server_url));

    let seed = move || {
        let api = api();
        let token = current_token(&state);
        let mut orders = state.orders;
        spawn(async move {
            let Some(token) = token else { return };
            if let Ok(resp) = api.list_orders(&token).await {
                orders.write().seed(&resp.orders);
            }
        });
    };

    use_future(move || {
        seed();
        async move {}
    });

    let mut exchange = use_signal(|| SUPPORTED_EXCHANGES[0].to_string());
    let mut pair = use_signal(String::new);
    let mut side = use_signal(|| SIDES[0].to_string());
    let mut order_type = use_signal(|| ORDER_TYPES[0].to_string());
    let mut quantity = use_signal(String::new);
    let mut price = use_signal(String::new);
    let mut place_error = use_signal(|| None::<String>);
    let mut cancel_error = use_signal(|| None::<String>);
    let mut breaker_message = use_signal(|| None::<String>);

    let on_place = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let req = PlaceOrderRequest {
            exchange: exchange(),
            pair: pair(),
            side: side(),
            order_type: order_type(),
            quantity: quantity(),
            price: {
                let p = price();
                if p.is_empty() {
                    None
                } else {
                    Some(p)
                }
            },
            strategy_id: None,
        };
        spawn(async move {
            match api.place_order(&token, req).await {
                Ok(_) => {
                    place_error.set(None);
                    seed();
                }
                Err(e) => place_error.set(Some(e)),
            }
        });
    };

    let on_cancel = move |client_order_id: String| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            match api.cancel_order(&token, &client_order_id).await {
                Ok(()) => seed(),
                Err(e) => cancel_error.set(Some(e)),
            }
        });
    };

    let on_reset_breaker = move |_| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            match api.reset_circuit_breaker(&token).await {
                Ok(()) => breaker_message.set(Some("Circuit breaker reset".to_string())),
                Err(e) => breaker_message.set(Some(e)),
            }
        });
    };

    let order_rows = state.orders.read().rows().to_vec();

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Orders" }
                div { class: "page-sub", "Manual orders + live fills from strategies" }
            }
            button { class: "btn btn-danger", onclick: on_reset_breaker, "Reset circuit breaker" }
        }

        if let Some(msg) = breaker_message() {
            div { class: "form-success", "{msg}" }
        }

        section { class: "block card",
            span { class: "label", style: "margin-bottom:14px;", "Place Order" }
            form { onsubmit: on_place,
                div { class: "field-row grid-4",
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
                        label { "Side" }
                        select {
                            class: "finput",
                            value: "{side}",
                            onchange: move |e| side.set(e.value()),
                            for s in SIDES { option { value: *s, "{s}" } }
                        }
                    }
                    div { class: "field",
                        label { "Type" }
                        select {
                            class: "finput",
                            value: "{order_type}",
                            onchange: move |e| order_type.set(e.value()),
                            for t in ORDER_TYPES { option { value: *t, "{t}" } }
                        }
                    }
                }
                div { class: "field-row grid-2", style: "margin-top:10px; margin-bottom:14px;",
                    div { class: "field",
                        label { "Quantity" }
                        input { class: "finput", placeholder: "0.01", value: "{quantity}", oninput: move |e| quantity.set(e.value()) }
                    }
                    div { class: "field",
                        label { "Price (limit only)" }
                        input { class: "finput", placeholder: "optional", value: "{price}", oninput: move |e| price.set(e.value()) }
                    }
                }
                if let Some(err) = place_error() {
                    div { class: "form-error", "{err}" }
                }
                button { class: "btn btn-primary", r#type: "submit", "Place order" }
            }
        }

        section { class: "block",
            span { class: "label", "Order History" }
            if let Some(err) = cancel_error() {
                div { class: "form-error", "{err}" }
            }
            div { class: "table-wrap",
                table {
                    thead {
                        tr {
                            th { "Time" } th { "Exchange" } th { "Pair" } th { "Side" } th { "Qty" }
                            th { "Status" } th { "Fill Price" } th { "Strategy" } th { "" }
                        }
                    }
                    tbody {
                        for row in order_rows.iter() {
                            tr { key: "{row.client_order_id}",
                                td { class: "mono", "{row.time}" }
                                td { "{row.exchange}" }
                                td { class: "mono", "{row.pair}" }
                                td {
                                    span {
                                        class: if row.side == "buy" { "pill pill-green" } else { "pill pill-red" },
                                        "{row.side}"
                                    }
                                }
                                td { class: "mono", "{row.quantity}" }
                                td {
                                    span {
                                        class: if row.status == "filled" { "pill pill-green" } else if row.status == "open" { "pill pill-yellow" } else { "pill pill-muted" },
                                        "{row.status}"
                                    }
                                }
                                td { class: "mono", "{row.fill_price.clone().unwrap_or_else(|| \"—\".to_string())}" }
                                td { "{row.strategy_id.clone().unwrap_or_else(|| \"manual\".to_string())}" }
                                td {
                                    if row.status == "open" {
                                        button {
                                            class: "btn btn-danger btn-sm",
                                            onclick: {
                                                let id = row.client_order_id.clone();
                                                move |_| on_cancel(id.clone())
                                            },
                                            "Cancel"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
