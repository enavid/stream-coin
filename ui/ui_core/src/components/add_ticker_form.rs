use dioxus::prelude::*;

use crate::state::AppState;

/// Inline-expanding "add ticker" card: collapsed it's a dashed `+` tile;
/// clicking it reveals an exchange/pair form in place (no modal).
///
/// Exchange and pair options come from `AppState.catalog` (populated by
/// `AppShell` from `GET /v1/exchanges` / `GET /v1/exchanges/{name}/pairs`)
/// rather than a hardcoded list — whichever exchanges/pairs are actually
/// enabled in the engine's registry are what gets suggested here.
#[component]
pub fn AddTickerForm(on_start: EventHandler<(String, String)>) -> Element {
    let state = use_context::<AppState>();
    let mut expanded = use_signal(|| false);
    let mut exchange_choice = use_signal(String::new);
    let mut pair_choice = use_signal(String::new);

    let catalog = state.catalog.read();
    let exchanges = catalog.exchanges().to_vec();
    let selected_exchange = catalog.resolve_exchange(&exchange_choice());
    let pairs = catalog.pairs_for(&selected_exchange).to_vec();
    let selected_pair = catalog.resolve_pair(&selected_exchange, &pair_choice());
    drop(catalog);

    rsx! {
        div {
            class: if expanded() { "add-card expanded" } else { "add-card" },
            onclick: move |_| {
                if !expanded() {
                    expanded.set(true);
                }
            },

            if !expanded() {
                div { class: "add-trigger",
                    div { class: "add-icon", "+" }
                    div { "Add Ticker" }
                }
            } else {
                div {
                    class: "add-form",
                    onclick: move |evt: Event<MouseData>| evt.stop_propagation(),

                    if exchanges.is_empty() {
                        div { style: "font-size:11px; color:var(--muted2); padding:4px 0;",
                            "No exchanges available"
                        }
                    } else {
                        select {
                            class: "finput",
                            value: "{selected_exchange}",
                            onchange: move |evt| {
                                exchange_choice.set(evt.value());
                                pair_choice.set(String::new());
                            },
                            for ex in exchanges.iter() {
                                option { value: "{ex.name}", "{ex.name}" }
                            }
                        }
                        if pairs.is_empty() {
                            div { style: "font-size:11px; color:var(--muted2); padding:4px 0;",
                                "No active pairs for this exchange"
                            }
                        } else {
                            select {
                                class: "finput",
                                value: "{selected_pair}",
                                onchange: move |evt| pair_choice.set(evt.value()),
                                for p in pairs.iter() {
                                    option { value: "{p.base}/{p.quote}", "{p.base}/{p.quote}" }
                                }
                            }
                        }
                    }
                    div { class: "btn-row",
                        button {
                            class: "btn-cancel",
                            onclick: move |_| expanded.set(false),
                            "Cancel"
                        }
                        button {
                            class: "btn-start",
                            onclick: move |_| {
                                if !selected_exchange.is_empty() && !selected_pair.is_empty() {
                                    on_start.call((selected_exchange.clone(), selected_pair.clone()));
                                    pair_choice.set(String::new());
                                    expanded.set(false);
                                }
                            },
                            "Start"
                        }
                    }
                }
            }
        }
    }
}
