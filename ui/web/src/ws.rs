use dioxus::prelude::*;
use futures_util::StreamExt;
use gloo_net::websocket::futures::WebSocket;
use gloo_net::websocket::Message;

use ui_core::api::ApiClient;
use ui_core::protocol::WsEvent;
use ui_core::state::AppState;

const RECONNECT_DELAY_MS: u32 = 2_000;

/// How long a ticker card's up/down flash stays lit after a tick. Long
/// enough to register as "this just moved," short enough that it's
/// visibly transient rather than reading as a permanent state — see
/// `TickerStore::clear_flash`.
const FLASH_DURATION_MS: u32 = 900;

/// Connects to the backend's `/v1/ws` feed and applies every price/signal/
/// order message to `state`, reconnecting with a fixed delay on disconnect.
/// `/v1/ws` requires a JWT like every other route, but a browser's native
/// WebSocket API can't set an `Authorization` header on the upgrade
/// request, so the token rides in the URL instead
/// ([`ApiClient::ws_url_with_token`]). Until a session exists (e.g. the
/// user hasn't logged in yet), this waits without attempting a
/// connection at all — connecting with no token would just fail the
/// handshake every `RECONNECT_DELAY_MS` for no reason.
///
/// This is the only platform-specific piece of the dashboard: a future
/// desktop/mobile crate provides its own version of this function (e.g.
/// backed by `tokio-tungstenite`) and otherwise reuses every component,
/// `AppState`, and protocol type from `ui_core` unchanged.
pub async fn connect_and_listen(api: ApiClient, mut state: AppState) {
    loop {
        let token = state.session.read().as_ref().map(|s| s.token.clone());
        let Some(token) = token else {
            gloo_timers::future::TimeoutFuture::new(RECONNECT_DELAY_MS).await;
            continue;
        };

        if let Ok(mut socket) = WebSocket::open(&api.ws_url_with_token(&token)) {
            state.set_connected(true);

            while let Some(msg) = socket.next().await {
                match msg {
                    Ok(Message::Text(text)) => match WsEvent::parse(&text) {
                        Ok(WsEvent::PriceUpdate(price)) => {
                            state.apply_price(&price);
                            let key = price.ticker_key();
                            let mut state = state;
                            spawn(async move {
                                gloo_timers::future::TimeoutFuture::new(FLASH_DURATION_MS).await;
                                state.clear_flash(&key);
                            });
                        }
                        Ok(WsEvent::Signal(signal)) => state.apply_signal(&signal),
                        Ok(WsEvent::OrderUpdate(order)) => state.apply_order_update(&order),
                        Ok(WsEvent::Candle(candle)) => state.apply_candle(&candle),
                        Err(_) => {}
                    },
                    Ok(Message::Bytes(_)) => {}
                    Err(_) => break,
                }
            }
        }

        state.set_connected(false);
        gloo_timers::future::TimeoutFuture::new(RECONNECT_DELAY_MS).await;
    }
}
