use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc::Sender;
use tokio::task::AbortHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::exchange::entity::ExchangeId;
use crate::exchange::port::{ExchangeAdapter, ExchangeAdapterError};
use crate::price::entity::{Price, TradingPair};

const WS_URL: &str = "wss://api1.tabdeal.org/stream/";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub struct TabdealWsAdapter {
    ws_url: &'static str,
    reconnect_delay: Duration,
}

impl TabdealWsAdapter {
    pub fn new() -> Self {
        Self {
            ws_url: WS_URL,
            reconnect_delay: RECONNECT_DELAY,
        }
    }
}

impl Default for TabdealWsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TabdealWsAdapter {
    pub fn build_subscribe_message(symbol: &str) -> Value {
        json!({
            "method": "SUBSCRIBE",
            "params": [format!("{}@depth@2000ms", symbol)],
            "id": 1
        })
    }

    pub fn parse_depth_message(msg: &Value) -> Result<Price, String> {
        let data = &msg["data"];

        let symbol = data["s"]
            .as_str()
            .ok_or_else(|| "missing symbol field".to_string())?;

        let bids = data["b"]
            .as_array()
            .ok_or_else(|| "missing bids".to_string())?;
        if bids.is_empty() {
            return Err("empty bids".to_string());
        }

        let asks = data["a"]
            .as_array()
            .ok_or_else(|| "missing asks".to_string())?;
        if asks.is_empty() {
            return Err("empty asks".to_string());
        }

        let bid = bids[0][0]
            .as_str()
            .ok_or_else(|| "invalid bid price".to_string())
            .and_then(Self::parse_price_units)?;

        let ask = asks[0][0]
            .as_str()
            .ok_or_else(|| "invalid ask price".to_string())
            .and_then(Self::parse_price_units)?;

        Ok(Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: Self::parse_trading_pair(symbol),
            bid,
            ask,
            timestamp: Utc::now(),
        })
    }

    /// Parses a price string from Tabdeal's WS API into rial units.
    /// Tabdeal prices are already in IRR; fractional rials are truncated (not rounded).
    fn parse_price_units(s: &str) -> Result<u64, String> {
        let v = s.parse::<f64>().map_err(|e| e.to_string())?;
        if !v.is_finite() {
            return Err(format!("price is not finite: {s}"));
        }
        if v < 0.0 {
            return Err(format!("price must be non-negative: {s}"));
        }
        Ok(v as u64)
    }

    fn parse_trading_pair(symbol: &str) -> TradingPair {
        if let Some(base) = symbol.strip_suffix("IRT") {
            TradingPair::new(base, "IRT")
        } else if let Some(base) = symbol.strip_suffix("USDT") {
            TradingPair::new(base, "USDT")
        } else {
            TradingPair::new(symbol, "")
        }
    }
}

#[async_trait]
impl ExchangeAdapter for TabdealWsAdapter {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("tabdeal")
    }

    fn symbol_for_pair(&self, pair: &TradingPair) -> String {
        format!("{}{}", pair.base, pair.quote).to_lowercase()
    }

    async fn subscribe(
        &self,
        pair: &TradingPair,
        tx: Sender<Price>,
    ) -> Result<AbortHandle, ExchangeAdapterError> {
        let symbol = self.symbol_for_pair(pair);
        let subscribe_msg = Self::build_subscribe_message(&symbol).to_string();

        let url = self.ws_url;
        let reconnect_delay = self.reconnect_delay;
        let handle = tokio::spawn(async move {
            loop {
                match connect_async(url).await {
                    Ok((mut ws, _)) => {
                        tracing::info!(symbol = %symbol, "tabdeal websocket connected");

                        if ws.send(Message::Text(subscribe_msg.clone())).await.is_err() {
                            tracing::error!(symbol = %symbol, "failed to send subscribe message");
                            tokio::time::sleep(reconnect_delay).await;
                            continue;
                        }

                        while let Some(msg) = ws.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                        if let Ok(price) =
                                            TabdealWsAdapter::parse_depth_message(&json)
                                        {
                                            if tx.send(price).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => break,
                                Err(e) => {
                                    tracing::warn!(symbol = %symbol, error = %e, "websocket error");
                                    break;
                                }
                                _ => {}
                            }
                        }

                        tracing::warn!(symbol = %symbol, "tabdeal websocket disconnected, reconnecting");
                    }
                    Err(e) => {
                        tracing::error!(symbol = %symbol, error = %e, "failed to connect to tabdeal");
                    }
                }

                tokio::time::sleep(reconnect_delay).await;
            }
        });

        Ok(handle.abort_handle())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    fn depth_message(symbol: &str, bid: &str, ask: &str) -> Value {
        json!({
            "stream": format!("{}@depth@2000ms", symbol.to_lowercase()),
            "data": {
                "e": "depthUpdate",
                "E": 1657530675579u64,
                "s": symbol,
                "b": [[bid, "1.0"]],
                "a": [[ask, "1.0"]]
            }
        })
    }

    #[test]
    fn parse_depth_message_extracts_bid_price() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 58000);
    }

    #[test]
    fn parse_depth_message_extracts_ask_price() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.ask, 58100);
    }

    #[test]
    fn parse_depth_message_sets_exchange_as_tabdeal() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.exchange.to_string(), "tabdeal");
    }

    #[test]
    fn parse_depth_message_extracts_irt_trading_pair() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.pair.base, "USDT");
        assert_eq!(price.pair.quote, "IRT");
    }

    #[test]
    fn parse_depth_message_extracts_usdt_trading_pair() {
        let msg = depth_message("BTCUSDT", "65000", "65100");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.pair.base, "BTC");
        assert_eq!(price.pair.quote, "USDT");
    }

    #[test]
    fn parse_depth_message_fails_when_bids_is_empty() {
        let msg = json!({
            "stream": "usdtirt@depth@2000ms",
            "data": {
                "s": "USDTIRT",
                "b": [],
                "a": [["58100", "1.0"]]
            }
        });
        assert!(TabdealWsAdapter::parse_depth_message(&msg).is_err());
    }

    #[test]
    fn build_subscribe_message_has_subscribe_method() {
        let msg = TabdealWsAdapter::build_subscribe_message("usdtirt");
        assert_eq!(msg["method"], "SUBSCRIBE");
    }

    #[test]
    fn build_subscribe_message_includes_depth_stream_for_symbol() {
        let msg = TabdealWsAdapter::build_subscribe_message("usdtirt");
        assert_eq!(msg["params"][0], "usdtirt@depth@2000ms");
    }

    // --- adapter lifecycle / reconnect tests ---

    fn depth_json(bid: &str, ask: &str) -> String {
        format!(
            r#"{{"stream":"usdtirt@depth@2000ms","data":{{"e":"depthUpdate","E":1657530675579,"s":"USDTIRT","b":[["{bid}","1.0"]],"a":[["{ask}","1.0"]]}}}}"#
        )
    }

    async fn start_test_ws_server(
        connect_count: Arc<AtomicUsize>,
        behavior: impl Fn(
                tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + Sync
            + 'static,
    ) -> std::net::SocketAddr {
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let behavior = Arc::new(behavior);

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                connect_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let ws = accept_async(stream).await.unwrap();
                let beh = Arc::clone(&behavior);
                tokio::spawn(beh(ws));
            }
        });

        addr
    }

    #[tokio::test]
    async fn adapter_delivers_price_through_channel() {
        use std::time::Duration;
        use tokio_tungstenite::tungstenite::Message;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |mut ws| {
            Box::pin(async move {
                use futures_util::SinkExt;
                let _ = ws.send(Message::Text(depth_json("58000", "58100"))).await;
                let _ = ws.send(Message::Close(None)).await;
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = TabdealWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let _handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

        let price = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("timed out waiting for price")
            .expect("channel closed before price arrived");

        assert_eq!(price.bid, 58000);
        assert_eq!(price.ask, 58100);
    }

    #[tokio::test]
    async fn adapter_reconnects_after_server_closes_connection() {
        use std::time::Duration;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |mut ws| {
            Box::pin(async move {
                use futures_util::SinkExt;
                // immediately close — adapter must reconnect
                let _ = ws
                    .send(tokio_tungstenite::tungstenite::Message::Close(None))
                    .await;
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = TabdealWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

        // Allow time for at least 2 connect attempts (each with a 5s backoff
        // but the server closes immediately so the cycle is fast).
        tokio::time::sleep(Duration::from_millis(200)).await;

        handle.abort();

        assert!(
            connections.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "adapter must reconnect after a clean server close"
        );
    }

    #[tokio::test]
    async fn adapter_stops_on_abort_handle() {
        use std::time::Duration;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |ws| {
            Box::pin(async move {
                // hold the connection open indefinitely
                futures_util::StreamExt::for_each(ws, |_| async {}).await;
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = TabdealWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(10);

        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();

        // After abort the task must finish (not keep reconnecting).
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_at_abort = connections.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_after = connections.load(std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            count_at_abort, count_after,
            "adapter must not reconnect after abort"
        );
    }

    #[tokio::test]
    async fn adapter_stops_when_price_channel_receiver_is_dropped() {
        use std::time::Duration;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |mut ws| {
            Box::pin(async move {
                use futures_util::SinkExt;
                // send prices in a tight loop
                loop {
                    if ws
                        .send(tokio_tungstenite::tungstenite::Message::Text(depth_json(
                            "58000", "58100",
                        )))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = TabdealWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

        // Let the adapter start and receive at least one price
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Drop the receiver — adapter must exit its task
        drop(rx);
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Task should be finished; abort is a no-op if already done
        handle.abort();

        // Verify the connection count doesn't grow after rx drop
        // (no reconnect loop running)
        let count_before = connections.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_after = connections.load(std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            count_before, count_after,
            "adapter must not reconnect after the price channel is closed"
        );
    }

    // --- malformed / boundary price tests ---

    #[test]
    fn parse_depth_message_nan_bid_returns_error() {
        let msg = depth_message("USDTIRT", "NaN", "58100");
        assert!(
            TabdealWsAdapter::parse_depth_message(&msg).is_err(),
            "NaN bid must return Err, not a silently zeroed price"
        );
    }

    #[test]
    fn parse_depth_message_infinity_ask_returns_error() {
        let msg = depth_message("USDTIRT", "58000", "inf");
        assert!(
            TabdealWsAdapter::parse_depth_message(&msg).is_err(),
            "infinite ask must return Err, not u64::MAX"
        );
    }

    #[test]
    fn parse_depth_message_negative_bid_returns_error() {
        let msg = depth_message("USDTIRT", "-100", "58100");
        assert!(
            TabdealWsAdapter::parse_depth_message(&msg).is_err(),
            "negative bid must return Err, not a silently zeroed price"
        );
    }

    #[test]
    fn parse_depth_message_negative_ask_returns_error() {
        let msg = depth_message("USDTIRT", "58000", "-200");
        assert!(
            TabdealWsAdapter::parse_depth_message(&msg).is_err(),
            "negative ask must return Err, not a silently zeroed price"
        );
    }

    #[test]
    fn parse_depth_message_zero_prices_succeeds() {
        let msg = depth_message("USDTIRT", "0", "0");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 0);
        assert_eq!(price.ask, 0);
    }

    #[test]
    fn parse_depth_message_decimal_price_truncates_to_units() {
        let msg = depth_message("USDTIRT", "58000.9", "58100.1");
        let price = TabdealWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 58000);
        assert_eq!(price.ask, 58100);
    }

    // --- symbol_for_pair unit tests ---

    #[test]
    fn symbol_for_pair_usdt_irt() {
        let adapter = TabdealWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("USDT", "IRT")),
            "usdtirt"
        );
    }

    #[test]
    fn symbol_for_pair_btc_irt() {
        let adapter = TabdealWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("BTC", "IRT")),
            "btcirt"
        );
    }

    #[test]
    fn symbol_for_pair_eth_usdt() {
        let adapter = TabdealWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("ETH", "USDT")),
            "ethusdt"
        );
    }

    #[test]
    fn symbol_for_pair_is_always_lowercase() {
        let adapter = TabdealWsAdapter::new();
        let result = adapter.symbol_for_pair(&TradingPair::new("BTC", "IRT"));
        assert!(
            !result.chars().any(|c| c.is_uppercase()),
            "result must be all lowercase, got: {result}"
        );
    }

    // --- parse_price_units unit tests ---

    #[test]
    fn parse_price_units_truncates_fractional_rials() {
        assert_eq!(
            TabdealWsAdapter::parse_price_units("58000.9").unwrap(),
            58000
        );
    }

    #[test]
    fn parse_price_units_zero_is_valid() {
        assert_eq!(TabdealWsAdapter::parse_price_units("0").unwrap(), 0);
    }

    #[test]
    fn parse_price_units_zero_point_zero_is_valid() {
        assert_eq!(TabdealWsAdapter::parse_price_units("0.0").unwrap(), 0);
    }

    #[test]
    fn parse_price_units_integer_string_is_valid() {
        assert_eq!(
            TabdealWsAdapter::parse_price_units("175500").unwrap(),
            175500
        );
    }

    #[test]
    fn parse_price_units_large_value_near_u64_max() {
        assert!(TabdealWsAdapter::parse_price_units("18000000000000000000").is_ok());
    }

    #[test]
    fn parse_price_units_negative_returns_err() {
        assert!(TabdealWsAdapter::parse_price_units("-1").is_err());
    }

    #[test]
    fn parse_price_units_nan_string_returns_err() {
        assert!(TabdealWsAdapter::parse_price_units("NaN").is_err());
    }

    #[test]
    fn parse_price_units_infinity_string_returns_err() {
        assert!(TabdealWsAdapter::parse_price_units("Infinity").is_err());
    }

    #[test]
    fn parse_price_units_non_numeric_string_returns_err() {
        assert!(TabdealWsAdapter::parse_price_units("abc").is_err());
    }

    #[test]
    fn parse_price_units_empty_string_returns_err() {
        assert!(TabdealWsAdapter::parse_price_units("").is_err());
    }
}
