use dioxus::prelude::*;
use futures_util::StreamExt;
use gloo_net::websocket::futures::WebSocket;
use gloo_net::websocket::Message;

use ui_core::api::ApiClient;
use ui_core::protocol::WsEvent;
use ui_core::reconnect::reconnect_delay_ms;
use ui_core::state::AppState;

/// How long to wait between polls while there's no session yet (the user
/// hasn't logged in) — not a reconnect backoff, just a cheap idle poll, so
/// it stays fixed rather than growing unbounded.
const NO_SESSION_POLL_MS: u32 = 2_000;

/// How long a ticker card's up/down flash stays lit after a tick. Long
/// enough to register as "this just moved," short enough that it's
/// visibly transient rather than reading as a permanent state — see
/// `TickerStore::clear_flash`.
const FLASH_DURATION_MS: u32 = 900;

/// Connects to the backend's `/v1/ws` feed and applies every price/signal/
/// order message to `state`, reconnecting with exponential backoff + jitter
/// on disconnect (`ui_core::reconnect::reconnect_delay_ms` —
/// `ROADMAP.md`'s API standard: 500ms start, capped at 30s). After a
/// reconnect that follows a previous disconnect, bumps `AppState::
/// resync_epoch` so pages re-fetch via REST instead of trusting whatever
/// local state survived the drop.
///
/// `/v1/ws` requires a JWT like every other route, but a browser's native
/// WebSocket API can't set an `Authorization` header on the upgrade
/// request, so the token rides in the URL instead
/// ([`ApiClient::ws_url_with_token`]). Until a session exists (e.g. the
/// user hasn't logged in yet), this waits without attempting a connection
/// at all — connecting with no token would just fail the handshake
/// repeatedly for no reason.
///
/// This is the only platform-specific piece of the dashboard: a future
/// desktop/mobile crate provides its own version of this function (e.g.
/// backed by `tokio-tungstenite`) and otherwise reuses every component,
/// `AppState`, and protocol type from `ui_core` unchanged.
pub async fn connect_and_listen(api: ApiClient, mut state: AppState) {
    let mut attempt: u32 = 0;
    let mut had_disconnected = false;

    loop {
        let token = state.session.read().as_ref().map(|s| s.token.clone());
        let Some(token) = token else {
            gloo_timers::future::TimeoutFuture::new(NO_SESSION_POLL_MS).await;
            continue;
        };

        match WebSocket::open(&api.ws_url_with_token(&token)) {
            Err(e) => {
                web_sys::console::error_1(&format!("ws: failed to open connection: {e}").into());
            }
            Ok(mut socket) => {
                state.set_connected(true);
                attempt = 0;
                if had_disconnected {
                    state.mark_resynced();
                }

                while let Some(msg) = socket.next().await {
                    match msg {
                        Ok(Message::Text(text)) => match WsEvent::parse(&text) {
                            Ok(WsEvent::PriceUpdate(price)) => {
                                state.apply_price(&price);
                                let key = price.ticker_key();
                                let mut state = state;
                                spawn(async move {
                                    gloo_timers::future::TimeoutFuture::new(FLASH_DURATION_MS)
                                        .await;
                                    state.clear_flash(&key);
                                });
                            }
                            Ok(WsEvent::Signal(signal)) => state.apply_signal(&signal),
                            Ok(WsEvent::OrderUpdate(order)) => state.apply_order_update(&order),
                            Ok(WsEvent::Candle(candle)) => state.apply_candle(&candle),
                            Err(e) => web_sys::console::error_1(
                                &format!("ws: failed to parse message: {e}; raw: {text}").into(),
                            ),
                        },
                        Ok(Message::Bytes(_)) => {}
                        Err(e) => {
                            web_sys::console::error_1(
                                &format!("ws: connection error: {e}").into(),
                            );
                            break;
                        }
                    }
                }
            }
        }

        state.set_connected(false);
        had_disconnected = true;
        let delay = reconnect_delay_ms(attempt, js_sys::Math::random());
        attempt = attempt.saturating_add(1);
        gloo_timers::future::TimeoutFuture::new(delay).await;
    }
}
