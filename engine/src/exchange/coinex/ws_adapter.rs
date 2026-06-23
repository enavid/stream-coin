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
use crate::price::entity::{MarketType, Price, TradingPair};

const WS_URL: &str = "wss://socket.coinex.com/v2/spot";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub struct CoinexWsAdapter {
    ws_url: String,
    reconnect_delay: Duration,
}

impl CoinexWsAdapter {
    pub fn new() -> Self {
        Self {
            ws_url: WS_URL.to_string(),
            reconnect_delay: RECONNECT_DELAY,
        }
    }

    pub fn with_url(url: String) -> Self {
        Self {
            ws_url: url,
            reconnect_delay: RECONNECT_DELAY,
        }
    }

    pub fn build_subscribe_message(symbol: &str) -> Value {
        json!({
            "method": "depth.subscribe",
            "params": { "market_list": [[symbol, 20, "0", true]] },
            "id": 1
        })
    }

    pub fn parse_depth_message(msg: &Value) -> Result<Price, String> {
        let data = &msg["data"];

        let market = data["market"]
            .as_str()
            .ok_or_else(|| "missing market field".to_string())?;

        let depth = &data["depth"];

        let bids = depth["bids"]
            .as_array()
            .ok_or_else(|| "missing bids".to_string())?;
        if bids.is_empty() {
            return Err("empty bids".to_string());
        }

        let asks = depth["asks"]
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
            exchange: ExchangeId::new("coinex"),
            pair: Self::parse_trading_pair(market),
            bid,
            ask,
            timestamp: Utc::now(),
        })
    }

    fn parse_price_units(s: &str) -> Result<u64, String> {
        if s.starts_with('-') {
            return Err(format!("price must be non-negative: {s}"));
        }
        let integer_part = s.split_once('.').map_or(s, |(int, _)| int);
        integer_part
            .parse::<u64>()
            .map_err(|_| format!("invalid price: {s}"))
    }

    fn parse_trading_pair(market: &str) -> TradingPair {
        if let Some(base) = market.strip_suffix("USDT") {
            TradingPair::new(base, "USDT")
        } else if let Some(base) = market.strip_suffix("USDC") {
            TradingPair::new(base, "USDC")
        } else if let Some(base) = market.strip_suffix("BTC") {
            TradingPair::new(base, "BTC")
        } else {
            TradingPair::new(market, "")
        }
    }
}

impl Default for CoinexWsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExchangeAdapter for CoinexWsAdapter {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("coinex")
    }

    fn symbol_for_pair(&self, pair: &TradingPair) -> String {
        let base = format!("{}{}", pair.base, pair.quote);
        match pair.market_type {
            MarketType::Spot => base,
            MarketType::Futures | MarketType::Swap => format!("{base}-PERP"),
        }
    }

    async fn subscribe(
        &self,
        pair: &TradingPair,
        tx: Sender<Price>,
    ) -> Result<AbortHandle, ExchangeAdapterError> {
        let symbol = self.symbol_for_pair(pair);
        let subscribe_msg = Self::build_subscribe_message(&symbol).to_string();

        let url = self.ws_url.clone();
        let reconnect_delay = self.reconnect_delay;
        let handle = tokio::spawn(async move {
            loop {
                match connect_async(url.as_str()).await {
                    Ok((mut ws, _)) => {
                        tracing::info!(symbol = %symbol, "coinex websocket connected");

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
                                            CoinexWsAdapter::parse_depth_message(&json)
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

                        tracing::warn!(symbol = %symbol, "coinex websocket disconnected, reconnecting");
                    }
                    Err(e) => {
                        tracing::error!(symbol = %symbol, error = %e, "failed to connect to coinex");
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

    fn depth_update(market: &str, bid: &str, ask: &str) -> Value {
        json!({
            "method": "depth.update",
            "data": {
                "market": market,
                "is_full": true,
                "depth": {
                    "bids": [[bid, "1.0"]],
                    "asks": [[ask, "1.0"]],
                    "updated_at": 1657530675579u64
                }
            }
        })
    }

    // --- symbol_for_pair unit tests ---

    #[test]
    fn symbol_for_pair_usdt_market_is_uppercase_no_separator() {
        let adapter = CoinexWsAdapter::new();
        let result = adapter.symbol_for_pair(&TradingPair::new("BTC", "USDT"));
        assert_eq!(result, "BTCUSDT");
    }

    #[test]
    fn symbol_for_pair_futures_appends_perp_suffix() {
        let adapter = CoinexWsAdapter::new();
        let pair = TradingPair::with_market_type("BTC", "USDT", MarketType::Futures);
        assert_eq!(adapter.symbol_for_pair(&pair), "BTCUSDT-PERP");
    }

    // --- parse_depth_message unit tests ---

    #[test]
    fn depth_update_extracts_top_of_book_bid_and_ask() {
        let msg = depth_update("BTCUSDT", "30000.00", "30001.00");
        let price = CoinexWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 30000);
        assert_eq!(price.ask, 30001);
    }

    #[test]
    fn parse_depth_message_sets_exchange_as_coinex() {
        let msg = depth_update("BTCUSDT", "30000.00", "30001.00");
        let price = CoinexWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.exchange.to_string(), "coinex");
    }

    #[test]
    fn parse_depth_message_extracts_usdt_trading_pair() {
        let msg = depth_update("BTCUSDT", "30000.00", "30001.00");
        let price = CoinexWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.pair.base, "BTC");
        assert_eq!(price.pair.quote, "USDT");
    }

    #[test]
    fn parse_depth_message_fails_when_bids_is_empty() {
        let msg = json!({
            "method": "depth.update",
            "data": {
                "market": "BTCUSDT",
                "depth": { "bids": [], "asks": [["30001.00", "1.0"]] }
            }
        });
        assert!(CoinexWsAdapter::parse_depth_message(&msg).is_err());
    }

    #[test]
    fn parse_depth_message_negative_bid_returns_error() {
        let msg = depth_update("BTCUSDT", "-100", "30001.00");
        assert!(
            CoinexWsAdapter::parse_depth_message(&msg).is_err(),
            "negative bid must return Err"
        );
    }

    #[test]
    fn parse_depth_message_decimal_price_truncates_to_units() {
        let msg = depth_update("BTCUSDT", "30000.9", "30001.1");
        let price = CoinexWsAdapter::parse_depth_message(&msg).unwrap();
        assert_eq!(price.bid, 30000);
        assert_eq!(price.ask, 30001);
    }

    // --- build_subscribe_message unit tests ---

    #[test]
    fn build_subscribe_message_has_depth_subscribe_method() {
        let msg = CoinexWsAdapter::build_subscribe_message("BTCUSDT");
        assert_eq!(msg["method"], "depth.subscribe");
    }

    #[test]
    fn build_subscribe_message_includes_market_in_market_list() {
        let msg = CoinexWsAdapter::build_subscribe_message("BTCUSDT");
        assert_eq!(msg["params"]["market_list"][0][0], "BTCUSDT");
    }

    // --- adapter lifecycle tests ---

    fn depth_json(bid: &str, ask: &str) -> String {
        format!(
            r#"{{"method":"depth.update","data":{{"market":"BTCUSDT","is_full":true,"depth":{{"bids":[["{bid}","1.0"]],"asks":[["{ask}","1.0"]],"updated_at":1657530675579}}}}}}"#
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

    #[tokio::test(flavor = "current_thread")]
    async fn adapter_delivers_price_through_channel() {
        use std::time::Duration;
        use tokio_tungstenite::tungstenite::Message;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |mut ws| {
            Box::pin(async move {
                use futures_util::SinkExt;
                let _ = ws.send(Message::Text(depth_json("30000", "30001"))).await;
                let _ = ws.send(Message::Close(None)).await;
            })
        })
        .await;

        let adapter = CoinexWsAdapter {
            ws_url: format!("ws://{addr}"),
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let _handle = adapter
            .subscribe(&TradingPair::new("BTC", "USDT"), tx)
            .await
            .unwrap();

        let price = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for price")
            .expect("channel closed before price arrived");

        assert_eq!(price.bid, 30000);
        assert_eq!(price.ask, 30001);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn coinex_adapter_reconnects_on_close() {
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

        let adapter = CoinexWsAdapter {
            ws_url: format!("ws://{addr}"),
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let handle = adapter
            .subscribe(&TradingPair::new("BTC", "USDT"), tx)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if connections.load(std::sync::atomic::Ordering::SeqCst) >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("adapter did not reconnect within 5s");

        handle.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adapter_stops_on_abort_handle() {
        use std::time::Duration;

        let connections = Arc::new(AtomicUsize::new(0));
        let addr = start_test_ws_server(Arc::clone(&connections), |ws| {
            Box::pin(async move {
                futures_util::StreamExt::for_each(ws, |_| async {}).await;
            })
        })
        .await;

        let adapter = CoinexWsAdapter {
            ws_url: format!("ws://{addr}"),
            reconnect_delay: Duration::from_millis(10),
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(10);

        let handle = adapter
            .subscribe(&TradingPair::new("BTC", "USDT"), tx)
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
}
