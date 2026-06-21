use dioxus::prelude::*;

use crate::domain::{format_price, format_spread};
use crate::state::FeedRow;

#[component]
pub fn LiveFeed(rows: Vec<FeedRow>) -> Element {
    rsx! {
        section {
            span { class: "label", "Live Feed" }
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
                                tr {
                                    key: "{row.key}",
                                    td { class: "td-time", "{row.time}" }
                                    td { class: "td-exch", "{row.exchange}" }
                                    td { class: "td-pair", "{row.pair}" }
                                    td { class: "td-bid", "{format_price(row.bid)}" }
                                    td { class: "td-ask", "{format_price(row.ask)}" }
                                    td { class: "spread-cell", "{format_spread(row.ask - row.bid)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
