use dioxus::prelude::*;

use crate::domain::{format_price, format_spread, Direction, Ticker};

#[component]
pub fn TickerCard(
    ticker: Ticker,
    flash: Option<Direction>,
    on_stop: EventHandler<String>,
) -> Element {
    let flash_class = match flash {
        Some(Direction::Up) => " up",
        Some(Direction::Down) => " down",
        _ => "",
    };
    let key = ticker.key();

    rsx! {
        div { class: "tcard{flash_class}",
            button {
                class: "stop-x",
                title: "Stop",
                onclick: move |_| on_stop.call(key.clone()),
                "✕"
            }
            div { class: "tcard-top",
                div { class: "tcard-pair", "{ticker.pair}" }
                div { class: "tcard-exch", "{ticker.exchange}" }
            }
            div { class: "tcard-prices",
                div { class: "price-line",
                    span { class: "lbl", "Bid" }
                    span { class: "val bid", "{format_price(ticker.bid)}" }
                }
                div { class: "price-line",
                    span { class: "lbl", "Ask" }
                    span { class: "val ask", "{format_price(ticker.ask)}" }
                }
            }
            div { class: "tcard-divider" }
            div { class: "spread-line",
                span { class: "lbl", "Spread" }
                span { class: "spread-badge", "{format_spread(ticker.spread())} {ticker.quote_currency()}" }
            }
        }
    }
}
