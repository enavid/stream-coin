use futures_util::StreamExt;
use gloo_net::websocket::futures::WebSocket;
use gloo_net::websocket::Message;

use ui_core::protocol::WsEvent;
use ui_core::state::AppState;

const RECONNECT_DELAY_MS: u32 = 2_000;

/// Connects to the backend's `/v1/ws` feed and applies every price
/// message to `state`, reconnecting with a fixed delay on disconnect.
///
/// This is the only platform-specific piece of the dashboard: a future
/// desktop/mobile crate provides its own version of this function (e.g.
/// backed by `tokio-tungstenite`) and otherwise reuses every component,
/// `AppState`, and protocol type from `ui_core` unchanged.
pub async fn connect_and_listen(ws_url: String, mut state: AppState) {
    loop {
        if let Ok(mut socket) = WebSocket::open(&ws_url) {
            state.set_connected(true);

            while let Some(msg) = socket.next().await {
                match msg {
                    Ok(Message::Text(text)) => match WsEvent::parse(&text) {
                        Ok(WsEvent::PriceUpdate(price)) => state.apply_price(&price),
                        Ok(WsEvent::Signal(signal)) => state.apply_signal(&signal),
                        Ok(WsEvent::OrderUpdate(order)) => state.apply_order_update(&order),
                        Ok(WsEvent::Candle(_)) | Err(_) => {}
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
