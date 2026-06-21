use std::collections::HashMap;

use dioxus::prelude::*;

use crate::domain::{format_price, format_spread, Direction};
use crate::state::FeedRow;

#[component]
pub fn LiveFeed(rows: Vec<FeedRow>, flashes: HashMap<String, Direction>) -> Element {
    rsx! {
        section {
            span { class: "label", "Live Feed" }
            div { class: "page-sub", style: "margin-bottom:10px;",
                "Latest tick per ticker — updates in place, doesn't scroll"
            }
            if rows.is_empty() {
                div { class: "card", style: "color:var(--text-dim); font-size:12.5px;",
                    "No ticks yet. Start a ticker above to see it here."
                }
            } else {
                div { class: "feed-wrap",
                    div { class: "feed-scroll",
                        table {
                            thead {
                                tr {
                                    th { "Time" }
                                    th { "Exchange" }
                                    th { "Pair" }
                                    th { "Bid" }
                                    th { "Ask" }
                                    th { "Spread" }
                                }
                            }
                            tbody {
                                for row in rows.iter() {
                                    {
                                        let flash = flashes.get(&row.key).copied();
                                        let bid_class = match flash {
                                            Some(Direction::Up) => "td-bid flash-up",
                                            Some(Direction::Down) => "td-bid flash-down",
                                            _ => "td-bid",
                                        };
                                        rsx! {
                                            tr {
                                                key: "{row.key}",
                                                td { class: "td-time", "{row.time}" }
                                                td { class: "td-exch", "{row.exchange}" }
                                                td { class: "td-pair", "{row.pair}" }
                                                td { class: "{bid_class}", "{format_price(row.bid)}" }
                                                td { class: "td-ask", "{format_price(row.ask)}" }
                                                td { class: "spread-cell", "{format_spread(row.ask - row.bid)} {row.quote_currency()}" }
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
}
