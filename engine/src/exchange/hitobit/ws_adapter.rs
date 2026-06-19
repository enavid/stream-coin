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

const WS_URL: &str = "wss://stream.hitobit.com:443";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub struct HitobitWsAdapter {
    ws_url: &'static str,
    reconnect_delay: Duration,
}

impl HitobitWsAdapter {
    pub fn new() -> Self {
        Self {
            ws_url: WS_URL,
            reconnect_delay: RECONNECT_DELAY,
        }
    }

    pub fn build_subscribe_message(symbol: &str) -> Value {
        json!({
            "method": "SUBSCRIBE",
            "params": [format!("{}@depth@100ms", symbol)],
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
            exchange: ExchangeId::new("hitobit"),
            pair: Self::parse_trading_pair(symbol),
            bid,
            ask,
            timestamp: Utc::now(),
        })
    }

    /// Parses a price string from Hitobit's WS API into rial units.
    /// Hitobit prices are in IRR; fractional rials are truncated (not rounded).
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

impl Default for HitobitWsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExchangeAdapter for HitobitWsAdapter {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("hitobit")
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
                        tracing::info!(symbol = %symbol, "hitobit websocket connected");

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
                                            HitobitWsAdapter::parse_depth_message(&json)
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

                        tracing::warn!(symbol = %symbol, "hitobit websocket disconnected, reconnecting");
                    }
                    Err(e) => {
                        tracing::error!(symbol = %symbol, error = %e, "failed to connect to hitobit");
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
            "stream": format!("{}@depth@100ms", symbol.to_lowercase()),
            "data": {
                "e": "depthUpdate",
                "E": 1657530675579u64,
                "s": symbol,
                "b": [[bid, "1.0"]],
                "a": [[ask, "1.0"]]
            }
        })
    }

    // --- symbol_for_pair unit tests (ROADMAP 1a) ---

    #[test]
    fn symbol_for_pair_usdt_irt() {
        let adapter = HitobitWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("USDT", "IRT")),
            "usdtirt"
        );
    }

    #[test]
    fn symbol_for_pair_is_always_lowercase() {
        let adapter = HitobitWsAdapter::new();
        let result = adapter.symbol_for_pair(&TradingPair::new("BTC", "IRT"));
        assert!(
            !result.chars().any(|c| c.is_uppercase()),
            "result must be all lowercase, got: {result}"
        );
    }

    #[test]
    fn symbol_for_pair_btc_irt() {
        let adapter = HitobitWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("BTC", "IRT")),
            "btcirt"
        );
    }

    #[test]
    fn symbol_for_pair_eth_usdt() {
        let adapter = HitobitWsAdapter::new();
        assert_eq!(
            adapter.symbol_for_pair(&TradingPair::new("ETH", "USDT")),
            "ethusdt"
        );
    }

    // --- parse_depth_message unit tests ---

    #[test]
    fn parse_depth_message_extracts_bid_price() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 58000);
    }

    #[test]
    fn parse_depth_message_extracts_ask_price() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.ask, 58100);
    }

    #[test]
    fn parse_depth_message_sets_exchange_as_hitobit() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.exchange.to_string(), "hitobit");
    }

    #[test]
    fn parse_depth_message_extracts_irt_trading_pair() {
        let msg = depth_message("USDTIRT", "58000", "58100");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.pair.base, "USDT");
        assert_eq!(price.pair.quote, "IRT");
    }

    #[test]
    fn parse_depth_message_extracts_usdt_trading_pair() {
        let msg = depth_message("BTCUSDT", "65000", "65100");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.pair.base, "BTC");
        assert_eq!(price.pair.quote, "USDT");
    }

    #[test]
    fn parse_depth_message_fails_when_bids_is_empty() {
        let msg = json!({
            "stream": "usdtirt@depth@100ms",
            "data": {
                "s": "USDTIRT",
                "b": [],
                "a": [["58100", "1.0"]]
            }
        });
        assert!(HitobitWsAdapter::parse_depth_message(&msg).is_err());
    }

    #[test]
    fn parse_depth_message_nan_bid_returns_error() {
        let msg = depth_message("USDTIRT", "NaN", "58100");
        assert!(
            HitobitWsAdapter::parse_depth_message(&msg).is_err(),
            "NaN bid must return Err"
        );
    }

    #[test]
    fn parse_depth_message_negative_bid_returns_error() {
        let msg = depth_message("USDTIRT", "-100", "58100");
        assert!(
            HitobitWsAdapter::parse_depth_message(&msg).is_err(),
            "negative bid must return Err"
        );
    }

    #[test]
    fn parse_depth_message_decimal_price_truncates_to_units() {
        let msg = depth_message("USDTIRT", "58000.9", "58100.1");
        let price = HitobitWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 58000);
        assert_eq!(price.ask, 58100);
    }

    // --- build_subscribe_message unit tests ---

    #[test]
    fn build_subscribe_message_has_subscribe_method() {
        let msg = HitobitWsAdapter::build_subscribe_message("usdtirt");
        assert_eq!(msg["method"], "SUBSCRIBE");
    }

    #[test]
    fn build_subscribe_message_includes_depth_stream_for_symbol() {
        let msg = HitobitWsAdapter::build_subscribe_message("usdtirt");
        assert_eq!(msg["params"][0], "usdtirt@depth@100ms");
    }

    // --- adapter lifecycle tests ---

    fn depth_json(bid: &str, ask: &str) -> String {
        format!(
            r#"{{"stream":"usdtirt@depth@100ms","data":{{"e":"depthUpdate","E":1657530675579,"s":"USDTIRT","b":[["{bid}","1.0"]],"a":[["{ask}","1.0"]]}}}}"#
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
        let adapter = HitobitWsAdapter {
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
                let _ = ws
                    .send(tokio_tungstenite::tungstenite::Message::Close(None))
                    .await;
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = HitobitWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

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
                futures_util::StreamExt::for_each(ws, |_| async {}).await;
            })
        })
        .await;

        let url: &'static str = Box::leak(format!("ws://{addr}").into_boxed_str());
        let adapter = HitobitWsAdapter {
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
        let adapter = HitobitWsAdapter {
            ws_url: url,
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let handle = adapter
            .subscribe(&TradingPair::new("USDT", "IRT"), tx)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(rx);
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.abort();

        let count_before = connections.load(std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let count_after = connections.load(std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            count_before, count_after,
            "adapter must not reconnect after the price channel is closed"
        );
    }
}
