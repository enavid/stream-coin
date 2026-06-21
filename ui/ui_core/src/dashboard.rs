use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::{Header, LiveFeed, TickerGrid};
use crate::pages::current_token;
use crate::state::AppState;

/// The dashboard's visual composition: header, ticker cards, live feed.
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
    let flashes = tickers
        .keys()
        .filter_map(|k| store.flash_for(k).map(|d| (k.clone(), d)))
        .collect();
    let feed = store.feed().to_vec();
    drop(store);

    let connected = (state.connected)();

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
        Header { connected, server_url: server_url.clone() }
        main {
            TickerGrid { tickers, flashes, on_stop, on_start }
            LiveFeed { rows: feed }
        }
    }
}
