use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::{LiveFeed, TickerGrid};
use crate::pages::current_token;
use crate::state::AppState;

/// The dashboard's visual composition: ticker cards + live feed. The
/// connection status and `stream-coin`/server-url identity already live
/// in `AppShell`'s topbar — duplicating them here in a second card was
/// pure noise, so this only renders what's specific to this page.
///
/// Reads [`AppState`] from context rather than providing it, so the
/// platform's root component owns context setup *and* the WebSocket
/// connection that feeds it (transport is platform-specific — see
/// `ui/web/src/ws.rs` — everything else here is shared).
#[component]
pub fn Dashboard(server_url: String) -> Element {
    let mut state = use_context::<AppState>();
    let api = use_signal(|| ApiClient::new(server_url.clone()));

    let store = state.store.read();
    let tickers = store.tickers().clone();
    let flashes: std::collections::HashMap<_, _> = tickers
        .keys()
        .filter_map(|k| store.flash_for(k).map(|d| (k.clone(), d)))
        .collect();
    let feed = store.feed().to_vec();
    let active_count = tickers.len();
    let exchange_count = tickers
        .values()
        .map(|t| t.exchange.clone())
        .collect::<std::collections::HashSet<_>>()
        .len();
    drop(store);

    let signal_count = state.signals.read().rows().len();
    let open_order_count = state
        .orders
        .read()
        .rows()
        .iter()
        .filter(|o| o.status == "open")
        .count();

    let on_stop = move |key: String| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            if let Some((exchange, pair)) = key.split_once(':') {
                let _ = api.stop_ticker(&token, exchange, pair).await;
            }
            state.remove_ticker(&key);
        });
    };

    let on_start = move |(exchange, symbol): (String, String)| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            // The resulting price update arrives over the WebSocket feed
            // and the card is created reactively — no local insert needed.
            let _ = api.start_ticker(&token, &exchange, &symbol).await;
        });
    };

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Dashboard" }
                div { class: "page-sub", "Live bid/ask across every ticker you've started" }
            }
        }
        div { class: "stat-row block",
            div { class: "stat-card",
                div { class: "stat-label", "Active Tickers" }
                div { class: "stat-value", "{active_count}" }
            }
            div { class: "stat-card",
                div { class: "stat-label", "Exchanges" }
                div { class: "stat-value", "{exchange_count}" }
            }
            div { class: "stat-card",
                div { class: "stat-label", "Signals Seen" }
                div { class: "stat-value", "{signal_count}" }
            }
            div { class: "stat-card",
                div { class: "stat-label", "Open Orders" }
                div { class: "stat-value", "{open_order_count}" }
            }
        }
        TickerGrid { tickers, flashes: flashes.clone(), on_stop, on_start }
        LiveFeed { rows: feed, flashes }
    }
}
