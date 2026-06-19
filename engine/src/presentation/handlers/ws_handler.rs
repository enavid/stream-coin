use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_ws::Message;
use futures_util::StreamExt;
use tokio::sync::broadcast::error::RecvError;

use crate::presentation::shared::app_state::AppState;

/// Upgrades the connection to a WebSocket. Forwards every message published
/// on `AppState::broadcaster` to this client, while answering pings and the
/// close handshake from the client's incoming stream.
pub async fn ws_index(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;
    let mut rx = state.broadcaster.subscribe();

    actix_web::rt::spawn(async move {
        loop {
            tokio::select! {
                incoming = msg_stream.next() => {
                    match incoming {
                        Some(Ok(Message::Ping(bytes))) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
                update = rx.recv() => {
                    match update {
                        Ok(text) => {
                            if session.text(text).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            }
        }
        let _ = session.close(None).await;
    });

    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_http::ws;
    use actix_web::web::Bytes;
    use actix_web::{test, web, App};
    use futures_util::{SinkExt, StreamExt};
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::ExchangeRegistry;
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};

    fn build_state() -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
        })
    }

    #[actix_web::test]
    async fn ws_rejects_plain_http_get_request() {
        let state = build_state();
        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/ws", web::get().to(ws_index)),
        )
        .await;

        let req = test::TestRequest::get().uri("/ws").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn ws_forwards_broadcast_message_to_connected_client() {
        let state = build_state();
        let broadcaster = state.broadcaster.clone();

        let mut srv = actix_test::start(move || {
            App::new()
                .app_data(state.clone())
                .route("/ws", web::get().to(ws_index))
        });

        let mut conn = srv.ws_at("/ws").await.unwrap();

        broadcaster
            .send(r#"{"exchange":"tabdeal","pair":"USDT/IRT"}"#.to_string())
            .unwrap();

        let item = conn.next().await.unwrap().unwrap();
        assert_eq!(
            item,
            ws::Frame::Text(Bytes::from_static(
                br#"{"exchange":"tabdeal","pair":"USDT/IRT"}"#
            ))
        );
    }

    #[actix_web::test]
    async fn ws_delivers_messages_to_multiple_connected_clients() {
        let state = build_state();
        let broadcaster = state.broadcaster.clone();

        let mut srv = actix_test::start(move || {
            App::new()
                .app_data(state.clone())
                .route("/ws", web::get().to(ws_index))
        });

        let mut conn_a = srv.ws_at("/ws").await.unwrap();
        let mut conn_b = srv.ws_at("/ws").await.unwrap();

        broadcaster.send("hello".to_string()).unwrap();

        let item_a = conn_a.next().await.unwrap().unwrap();
        let item_b = conn_b.next().await.unwrap().unwrap();
        assert_eq!(item_a, ws::Frame::Text(Bytes::from_static(b"hello")));
        assert_eq!(item_b, ws::Frame::Text(Bytes::from_static(b"hello")));
    }

    #[actix_web::test]
    async fn ws_lagged_receiver_skips_old_messages_and_stays_connected() {
        use std::time::Duration;

        let state = build_state();
        let broadcaster = state.broadcaster.clone();

        let mut srv = actix_test::start(move || {
            App::new()
                .app_data(state.clone())
                .route("/ws", web::get().to(ws_index))
        });

        let mut conn = srv.ws_at("/ws").await.unwrap();

        // Flood past the broadcast channel capacity (256) without yielding
        // so the ws_handler task cannot drain the channel first.
        // This forces RecvError::Lagged when the task eventually runs.
        for i in 0..300u32 {
            let _ = broadcaster.send(format!("{i}"));
        }

        // Yield to let the spawned ws_handler task run and hit Lagged.
        tokio::task::yield_now().await;

        // A message sent AFTER the lag must still reach the client.
        broadcaster.send("sentinel".to_string()).unwrap();

        let deadline = Duration::from_millis(500);
        loop {
            match tokio::time::timeout(deadline, conn.next()).await {
                Ok(Some(Ok(ws::Frame::Text(bytes)))) if bytes == "sentinel" => break,
                Ok(Some(Ok(_))) => continue,
                _ => panic!("client did not receive the post-lag sentinel message"),
            }
        }
    }

    #[actix_web::test]
    async fn ws_connection_closes_cleanly_when_client_disconnects() {
        let state = build_state();

        let mut srv = actix_test::start(move || {
            App::new()
                .app_data(state.clone())
                .route("/ws", web::get().to(ws_index))
        });

        let mut conn = srv.ws_at("/ws").await.unwrap();
        conn.send(ws::Message::Close(None)).await.unwrap();
        let item = conn.next().await.unwrap().unwrap();
        assert!(matches!(item, ws::Frame::Close(_)));
    }
}
