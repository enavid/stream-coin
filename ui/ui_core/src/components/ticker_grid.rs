use std::collections::HashMap;

use dioxus::prelude::*;

use super::add_ticker_form::AddTickerForm;
use super::ticker_card::TickerCard;
use crate::domain::{Direction, Ticker};

#[component]
pub fn TickerGrid(
    tickers: HashMap<String, Ticker>,
    flashes: HashMap<String, Direction>,
    on_stop: EventHandler<String>,
    on_start: EventHandler<(String, String)>,
) -> Element {
    rsx! {
        section {
            span { class: "label", "Active Tickers" }
            div { class: "cards-row",
                for (key, ticker) in tickers.into_iter() {
                    TickerCard {
                        key: "{key}",
                        ticker: ticker.clone(),
                        flash: flashes.get(&key).copied(),
                        on_stop,
                    }
                }
                AddTickerForm { on_start }
            }
        }
    }
}
