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

pub struct TabdealWsAdapter;

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

    async fn subscribe(
        &self,
        symbol: &str,
        tx: Sender<Price>,
    ) -> Result<AbortHandle, ExchangeAdapterError> {
        let symbol = symbol.to_lowercase();
        let subscribe_msg = Self::build_subscribe_message(&symbol).to_string();

        let handle = tokio::spawn(async move {
            loop {
                match connect_async(WS_URL).await {
                    Ok((mut ws, _)) => {
                        tracing::info!(symbol = %symbol, "tabdeal websocket connected");

                        if ws.send(Message::Text(subscribe_msg.clone())).await.is_err() {
                            tracing::error!(symbol = %symbol, "failed to send subscribe message");
                            tokio::time::sleep(Duration::from_secs(5)).await;
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

                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        Ok(handle.abort_handle())
    }
}

#[cfg(test)]
mod tests {
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
}
