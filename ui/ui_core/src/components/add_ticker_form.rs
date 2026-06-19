use dioxus::prelude::*;

use crate::domain::SUPPORTED_EXCHANGES;

/// Inline-expanding "add ticker" card: collapsed it's a dashed `+` tile;
/// clicking it reveals an exchange/symbol form in place (no modal).
#[component]
pub fn AddTickerForm(on_start: EventHandler<(String, String)>) -> Element {
    let mut expanded = use_signal(|| false);
    let mut exchange = use_signal(|| SUPPORTED_EXCHANGES[0].to_string());
    let mut symbol = use_signal(String::new);

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

                    select {
                        class: "finput",
                        value: "{exchange}",
                        onchange: move |evt| exchange.set(evt.value()),
                        for ex in SUPPORTED_EXCHANGES {
                            option { value: *ex, "{ex}" }
                        }
                    }
                    input {
                        class: "finput",
                        placeholder: "Symbol — e.g. USDTIRT",
                        value: "{symbol}",
                        oninput: move |evt| symbol.set(evt.value()),
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
                                let sym = symbol();
                                if !sym.trim().is_empty() {
                                    on_start.call((exchange(), sym));
                                    symbol.set(String::new());
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
