//! Wire format shared with the backend's `GET /v1/ws` broadcast feed.
//!
//! This mirrors `stream_coin::price::entity::Price`'s `Serialize` output
//! field-for-field. It's intentionally a small, hand-maintained mirror
//! rather than a shared crate dependency: the backend can't compile to
//! `wasm32` (it links `rdkafka`/`redis` native libs), so the contract is
//! kept here and locked in by the round-trip tests below.

use serde::{Deserialize, Serialize};

use crate::api::ClosedTrade;
use crate::domain::Ticker;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceMessage {
    pub exchange: String,
    pub pair: String,
    pub ask: u64,
    pub bid: u64,
    pub timestamp: String,
}

impl PriceMessage {
    pub fn parse(raw: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum WireMessage {
            PriceUpdate(PriceMessage),
        }
        let wire: WireMessage = serde_json::from_str(raw)?;
        match wire {
            WireMessage::PriceUpdate(msg) => Ok(msg),
        }
    }

    /// The same `"{exchange}:{pair}"` identity as `Ticker::key()`, computed
    /// straight from the wire message — lets the platform's WS transport
    /// (e.g. `ui/web/src/ws.rs`, to schedule a flash-clear timer) address a
    /// ticker without first building a `Ticker` just to throw it away.
    pub fn ticker_key(&self) -> String {
        Ticker::from(self).key()
    }
}

impl From<&PriceMessage> for Ticker {
    fn from(msg: &PriceMessage) -> Self {
        Ticker::new(&msg.exchange, &msg.pair, msg.bid as f64, msg.ask as f64)
    }
}

/// Mirrors the backend's `SignalPayload`
/// (`engine/src/presentation/shared/wire_message.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalMessage {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: String,
}

/// Mirrors the backend's `OrderUpdatePayload`. `quantity`/`fill_price` stay
/// `String` — the backend serializes `Decimal` as a string to avoid float
/// precision loss, and this UI never reparses them as `f64`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderUpdateMessage {
    pub order_id: String,
    pub client_order_id: String,
    pub exchange: String,
    pub pair: String,
    pub market_type: String,
    pub side: String,
    pub status: String,
    pub quantity: String,
    pub fill_price: Option<String>,
    pub strategy_id: Option<String>,
    pub timestamp: String,
}

/// Mirrors the backend's `CandlePayload` (`engine/src/candle/entity.rs`).
/// `time` stays a `String` (RFC3339, as chrono serializes it) — same
/// "never reparse the wire format" rule as `PriceMessage::timestamp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandleMessage {
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub time: String,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

impl CandleMessage {
    /// Same `"{exchange}:{pair}:{interval}"` key the engine's in-memory
    /// candle history buffer uses (`AppState::push_candle_history`).
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.exchange, self.pair, self.interval)
    }
}

/// Every variant of the backend's `WsMessage` that this UI cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    PriceUpdate(PriceMessage),
    Signal(SignalMessage),
    OrderUpdate(OrderUpdateMessage),
    Candle(CandleMessage),
    /// Loop 6h's `LiveTradeTracker` output — a live strategy's
    /// position close, same shape as a backtest's `ClosedTrade`. Lets the
    /// "Watch live" backtest toggle (`pages/backtest.rs`) feed the chart's
    /// existing trade overlay from a running strategy instead of a
    /// finished historical run.
    ClosedTrade(ClosedTrade),
}

impl WsEvent {
    pub fn parse(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_json() -> &'static str {
        r#"{"type":"price_update","exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#
    }

    #[test]
    fn parse_decodes_all_fields() {
        let msg = PriceMessage::parse(sample_json()).unwrap();
        assert_eq!(msg.exchange, "tabdeal");
        assert_eq!(msg.pair, "USDT/IRT");
        assert_eq!(msg.bid, 92815);
        assert_eq!(msg.ask, 92936);
        assert_eq!(msg.timestamp, "2026-06-18T10:26:33.123Z");
    }

    #[test]
    fn ticker_key_combines_exchange_and_pair_like_ticker_key() {
        let msg = PriceMessage::parse(sample_json()).unwrap();
        assert_eq!(msg.ticker_key(), "tabdeal:USDT/IRT");
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(PriceMessage::parse("not json").is_err());
    }

    #[test]
    fn parse_rejects_missing_required_field() {
        let missing_bid = r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"timestamp":"x"}"#;
        assert!(PriceMessage::parse(missing_bid).is_err());
    }

    #[test]
    fn round_trips_through_serialize_and_parse() {
        let original = PriceMessage {
            exchange: "nobitex".to_string(),
            pair: "BTC/IRT".to_string(),
            ask: 4_221_000_000,
            bid: 4_218_500_000,
            timestamp: "2026-06-18T10:00:00Z".to_string(),
        };
        // PriceMessage::serialize produces the payload fields (no type discriminator).
        // Re-wrap with the type tag to form the complete wire JSON before parsing.
        let payload_json = serde_json::to_string(&original).unwrap();
        let wire = format!(r#"{{"type":"price_update",{}"#, &payload_json[1..]);
        let parsed = PriceMessage::parse(&wire).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn converts_into_ticker_with_matching_exchange_and_pair() {
        let msg = PriceMessage::parse(sample_json()).unwrap();
        let ticker: Ticker = (&msg).into();
        assert_eq!(ticker.exchange, "tabdeal");
        assert_eq!(ticker.pair, "USDT/IRT");
        assert_eq!(ticker.bid, 92815.0);
        assert_eq!(ticker.ask, 92936.0);
    }

    // --- WS frame boundary tests ---

    #[test]
    fn parse_rejects_bid_as_string_type() {
        let json = r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":"not-a-number","timestamp":"2026-06-18T10:26:33Z"}"#;
        assert!(
            PriceMessage::parse(json).is_err(),
            "bid as a string must be rejected — field type is u64"
        );
    }

    #[test]
    fn parse_rejects_ask_as_null() {
        let json = r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":null,"bid":92815,"timestamp":"2026-06-18T10:26:33Z"}"#;
        assert!(
            PriceMessage::parse(json).is_err(),
            "null ask must be rejected — field is non-optional"
        );
    }

    #[test]
    fn parse_rejects_empty_string() {
        assert!(
            PriceMessage::parse("").is_err(),
            "empty string is not valid JSON"
        );
    }

    #[test]
    fn parse_rejects_json_array_at_root() {
        assert!(
            PriceMessage::parse("[]").is_err(),
            "a JSON array is not a PriceMessage object"
        );
    }

    #[test]
    fn price_message_parse_accepts_type_field_in_payload() {
        let json = r#"{"type":"price_update","exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#;
        assert!(
            PriceMessage::parse(json).is_ok(),
            "payload with type:price_update must parse successfully"
        );
    }

    #[test]
    fn price_message_parse_rejects_unknown_type() {
        let json = r#"{"type":"order_book","exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#;
        assert!(
            PriceMessage::parse(json).is_err(),
            "unknown type 'order_book' must be rejected"
        );
    }

    // --- WsEvent tests ---

    #[test]
    fn ws_event_parse_decodes_price_update() {
        let event = WsEvent::parse(sample_json()).unwrap();
        assert_eq!(
            event,
            WsEvent::PriceUpdate(PriceMessage::parse(sample_json()).unwrap())
        );
    }

    #[test]
    fn ws_event_parse_decodes_signal() {
        let json = r#"{"type":"signal","signal_id":"sig-1","strategy_id":"spread_threshold","exchange":"tabdeal","pair":"USDT/IRT","action":"buy","confidence":0.85,"timestamp":"2026-06-21T00:00:00Z"}"#;
        let event = WsEvent::parse(json).unwrap();
        assert_eq!(
            event,
            WsEvent::Signal(SignalMessage {
                signal_id: "sig-1".to_string(),
                strategy_id: "spread_threshold".to_string(),
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                action: "buy".to_string(),
                confidence: 0.85,
                timestamp: "2026-06-21T00:00:00Z".to_string(),
            })
        );
    }

    #[test]
    fn ws_event_parse_decodes_order_update() {
        let json = r#"{"type":"order_update","order_id":"ord-1","client_order_id":"cli-1","exchange":"tabdeal","pair":"USDT/IRT","market_type":"spot","side":"buy","status":"open","quantity":"100","fill_price":null,"strategy_id":null,"timestamp":"2026-06-21T00:00:00Z"}"#;
        let event = WsEvent::parse(json).unwrap();
        assert_eq!(
            event,
            WsEvent::OrderUpdate(OrderUpdateMessage {
                order_id: "ord-1".to_string(),
                client_order_id: "cli-1".to_string(),
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                market_type: "spot".to_string(),
                side: "buy".to_string(),
                status: "open".to_string(),
                quantity: "100".to_string(),
                fill_price: None,
                strategy_id: None,
                timestamp: "2026-06-21T00:00:00Z".to_string(),
            })
        );
    }

    #[test]
    fn ws_event_parse_keeps_quantity_as_string_not_number() {
        let json = r#"{"type":"order_update","order_id":"ord-1","client_order_id":"cli-1","exchange":"tabdeal","pair":"USDT/IRT","market_type":"spot","side":"buy","status":"filled","quantity":"0.004","fill_price":"4218500000","strategy_id":null,"timestamp":"2026-06-21T00:00:00Z"}"#;
        match WsEvent::parse(json).unwrap() {
            WsEvent::OrderUpdate(msg) => {
                assert_eq!(msg.quantity, "0.004");
                assert_eq!(msg.fill_price, Some("4218500000".to_string()));
            }
            other => panic!("expected OrderUpdate, got {other:?}"),
        }
    }

    #[test]
    fn ws_event_parse_decodes_closed_trade() {
        let json = r#"{"type":"closed_trade","strategy_id":"spread_threshold","side":"long","entry_price":58000,"exit_price":59000,"stop_loss":57000,"take_profit":60000,"quantity":1,"entry_time":"2026-06-21T00:00:00Z","exit_time":"2026-06-21T00:05:00Z","pnl":1000,"pnl_pct":1.72,"rr":1.0,"outcome":"win"}"#;
        let event = WsEvent::parse(json).unwrap();
        assert_eq!(
            event,
            WsEvent::ClosedTrade(crate::api::ClosedTrade {
                strategy_id: "spread_threshold".to_string(),
                side: crate::api::TradeSide::Long,
                entry_price: 58000,
                exit_price: 59000,
                stop_loss: Some(57000),
                take_profit: Some(60000),
                quantity: 1,
                entry_time: "2026-06-21T00:00:00Z".to_string(),
                exit_time: "2026-06-21T00:05:00Z".to_string(),
                pnl: 1000,
                pnl_pct: 1.72,
                rr: Some(1.0),
                outcome: crate::api::TradeOutcome::Win,
            })
        );
    }

    #[test]
    fn ws_event_parse_decodes_candle() {
        let json = r#"{"type":"candle","exchange":"tabdeal","pair":"USDT/IRT","interval":"1m","time":"2026-06-21T00:00:00Z","open":58000,"high":58500,"low":57800,"close":58200,"volume":0}"#;
        let event = WsEvent::parse(json).unwrap();
        assert_eq!(
            event,
            WsEvent::Candle(CandleMessage {
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1m".to_string(),
                time: "2026-06-21T00:00:00Z".to_string(),
                open: 58000,
                high: 58500,
                low: 57800,
                close: 58200,
                volume: 0,
            })
        );
    }

    #[test]
    fn candle_message_key_combines_exchange_pair_and_interval() {
        let msg = CandleMessage {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: "2026-06-21T00:00:00Z".to_string(),
            open: 1,
            high: 1,
            low: 1,
            close: 1,
            volume: 1,
        };
        assert_eq!(msg.key(), "tabdeal:USDT/IRT:1m");
    }

    #[test]
    fn ws_event_parse_rejects_unknown_type() {
        let json = r#"{"type":"order_book","exchange":"tabdeal"}"#;
        assert!(WsEvent::parse(json).is_err());
    }

    #[test]
    fn price_message_parse_rejects_missing_type_field() {
        let json = r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#;
        assert!(
            PriceMessage::parse(json).is_err(),
            "payload without type field must be rejected"
        );
    }
}
