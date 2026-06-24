use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::backtest::entity::ClosedTrade;
use crate::candle::entity::CandlePayload;
use crate::price::entity::Price;

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    PriceUpdate(PricePayload),
    Candle(CandlePayload),
    Signal(SignalPayload),
    OrderUpdate(OrderUpdatePayload),
    /// A live-preview trade close (Loop 6h's `LiveTradeTracker`) or a
    /// historical backtest's closed trade — same shape either way, so the
    /// chart's trade overlay (Phase 7) renders both through one code path.
    ClosedTrade(ClosedTrade),
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct OrderUpdatePayload {
    pub order_id: String,
    pub client_order_id: String,
    pub exchange: String,
    pub pair: String,
    pub market_type: String,
    pub side: String,
    /// "open" | "filled" | "partially_filled" | "cancelled" | "failed" | "dry_run"
    pub status: String,
    /// Decimal serialized as string — never f64.
    pub quantity: String,
    pub fill_price: Option<String>,
    pub strategy_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct SignalPayload {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    /// `#[serde(default)]` so signals from strategies (Python or otherwise)
    /// written before this field existed still deserialize without error.
    #[serde(default)]
    pub stop_loss: Option<u64>,
    #[serde(default)]
    pub take_profit: Option<u64>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct PricePayload {
    pub exchange: String,
    pub pair: String,
    pub bid: u64,
    pub ask: u64,
    pub timestamp: DateTime<Utc>,
}

impl From<&Price> for PricePayload {
    fn from(price: &Price) -> Self {
        PricePayload {
            exchange: price.exchange.to_string(),
            pair: price.pair.to_string(),
            bid: price.bid,
            ask: price.ask,
            timestamp: price.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::Value;

    use super::*;
    use crate::candle::entity::CandlePayload;
    use crate::exchange::entity::ExchangeId;
    use crate::price::entity::TradingPair;

    fn sample_payload() -> PricePayload {
        PricePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            bid: 175_500,
            ask: 175_520,
            timestamp: Utc::now(),
        }
    }

    fn sample_price() -> Price {
        Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRT"),
            bid: 175_500,
            ask: 175_520,
            timestamp: Utc::now(),
        }
    }

    fn sample_signal_payload() -> SignalPayload {
        SignalPayload {
            signal_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            strategy_id: "spread_threshold".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: "buy".to_string(),
            confidence: 0.85,
            timestamp: Utc::now(),
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn ws_message_price_update_serializes_with_type_field() {
        let msg = WsMessage::PriceUpdate(sample_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "price_update");
    }

    #[test]
    fn ws_message_type_is_snake_case_not_pascal() {
        let msg = WsMessage::PriceUpdate(sample_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            json["type"].as_str().unwrap(),
            "price_update",
            "type must be snake_case 'price_update', not 'PriceUpdate'"
        );
    }

    #[test]
    fn ws_message_fields_are_at_root_not_under_data_key() {
        let msg = WsMessage::PriceUpdate(sample_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(
            json["exchange"].is_string(),
            "exchange must be at root, not nested under a data key"
        );
        assert!(
            json["data"].is_null(),
            "there must be no 'data' wrapper key"
        );
    }

    #[test]
    fn ws_message_price_update_round_trips() {
        let original = WsMessage::PriceUpdate(sample_payload());
        let json = serde_json::to_string(&original).unwrap();
        let parsed: WsMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn ws_message_unknown_type_deserializes_to_error() {
        let json = r#"{"type":"order_book","exchange":"tabdeal","pair":"USDT/IRT","bid":1,"ask":2,"timestamp":"2026-06-19T00:00:00Z"}"#;
        let result: Result<WsMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown type must be rejected");
    }

    #[test]
    fn ws_message_missing_type_field_deserializes_to_error() {
        let json = r#"{"exchange":"tabdeal","pair":"USDT/IRT","bid":175500,"ask":175520,"timestamp":"2026-06-19T00:00:00Z"}"#;
        let result: Result<WsMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "missing type field must be rejected");
    }

    #[test]
    fn candle_payload_serializes_with_type_candle() {
        let payload = CandlePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: Utc::now(),
            open: 58000,
            high: 58500,
            low: 57800,
            close: 58200,
            volume: 0,
        };
        let msg = WsMessage::Candle(payload);
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            json["type"], "candle",
            "candle variant must serialize with type=candle"
        );
        assert!(json["exchange"].is_string(), "exchange must be at root");
        assert!(
            json["data"].is_null(),
            "fields must not be wrapped under a data key"
        );
        assert_eq!(json["interval"], "1m");
    }

    #[test]
    fn signal_serializes_with_type_signal() {
        let msg = WsMessage::Signal(sample_signal_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "signal");
    }

    #[test]
    fn signal_payload_contains_signal_id() {
        let msg = WsMessage::Signal(sample_signal_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(
            json["signal_id"].is_string(),
            "signal_id must be present and be a string"
        );
        assert!(
            !json["signal_id"].as_str().unwrap().is_empty(),
            "signal_id must not be empty"
        );
    }

    #[test]
    fn signal_fields_at_root_not_under_data_key() {
        let msg = WsMessage::Signal(sample_signal_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(json["signal_id"].is_string(), "signal_id must be at root");
        assert!(
            json["strategy_id"].is_string(),
            "strategy_id must be at root"
        );
        assert!(json["exchange"].is_string(), "exchange must be at root");
        assert!(
            json["data"].is_null(),
            "there must be no 'data' wrapper key"
        );
    }

    #[test]
    fn signal_payload_deserializes_without_stop_loss_take_profit_fields() {
        let json = r#"{"signal_id":"id","strategy_id":"s1","exchange":"tabdeal","pair":"USDT/IRT","action":"buy","confidence":0.9,"timestamp":"2026-06-20T00:00:00Z"}"#;
        let payload: SignalPayload = serde_json::from_str(json)
            .expect("old signal JSON without stop_loss/take_profit must still parse");
        assert!(payload.stop_loss.is_none());
        assert!(payload.take_profit.is_none());
    }

    #[test]
    fn signal_payload_round_trips_stop_loss_and_take_profit() {
        let mut payload = sample_signal_payload();
        payload.stop_loss = Some(173_460);
        payload.take_profit = Some(184_080);

        let json = serde_json::to_string(&payload).unwrap();
        let parsed: SignalPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.stop_loss, Some(173_460));
        assert_eq!(parsed.take_profit, Some(184_080));
    }

    #[test]
    fn signal_payload_serializes_stop_loss_as_number_not_string() {
        let mut payload = sample_signal_payload();
        payload.stop_loss = Some(100_000);
        let json: Value = serde_json::to_value(&payload).unwrap();
        assert!(json["stop_loss"].is_number());
    }

    fn sample_closed_trade() -> ClosedTrade {
        use crate::backtest::entity::TradeSide;
        ClosedTrade::close(
            "spread_threshold".to_string(),
            TradeSide::Long,
            100_000,
            110_000,
            Some(95_000),
            Some(120_000),
            1,
            Utc::now(),
            Utc::now(),
        )
    }

    #[test]
    fn closed_trade_message_serializes_with_type_closed_trade() {
        let msg = WsMessage::ClosedTrade(sample_closed_trade());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "closed_trade");
    }

    #[test]
    fn closed_trade_message_fields_are_at_root_not_under_data_key() {
        let msg = WsMessage::ClosedTrade(sample_closed_trade());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["entry_price"], 100_000);
        assert_eq!(json["exit_price"], 110_000);
        assert!(
            json["data"].is_null(),
            "there must be no 'data' wrapper key"
        );
    }

    #[test]
    fn closed_trade_message_round_trips_serialize_deserialize() {
        let msg = WsMessage::ClosedTrade(sample_closed_trade());
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: WsMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, parsed);
    }

    #[test]
    fn price_consumer_ignores_closed_trade_type_without_error() {
        let msg = WsMessage::ClosedTrade(sample_closed_trade());
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: WsMessage = serde_json::from_str(&json).unwrap();
        let is_price = matches!(parsed, WsMessage::PriceUpdate(_));
        assert!(
            !is_price,
            "a closed_trade message must not match the price_update variant"
        );
    }

    #[test]
    fn price_consumer_ignores_signal_type_without_error() {
        let json = r#"{"type":"signal","signal_id":"550e8400-e29b-41d4-a716-446655440000","strategy_id":"s1","exchange":"tabdeal","pair":"USDT/IRT","action":"buy","confidence":0.9,"timestamp":"2026-06-20T00:00:00Z"}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        let is_price = matches!(msg, WsMessage::PriceUpdate(_));
        assert!(
            !is_price,
            "a signal message must not match the price_update variant"
        );
    }

    fn sample_order_update_payload() -> OrderUpdatePayload {
        OrderUpdatePayload {
            order_id: "exchange-ord-001".to_string(),
            client_order_id: "client-uuid-001".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            market_type: "spot".to_string(),
            side: "buy".to_string(),
            status: "open".to_string(),
            quantity: "100".to_string(),
            fill_price: None,
            strategy_id: Some("spread-1".to_string()),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn order_update_payload_serializes_with_type_order_update() {
        let msg = WsMessage::OrderUpdate(sample_order_update_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "order_update");
    }

    #[test]
    fn order_update_fields_are_at_root_not_under_data_key() {
        let msg = WsMessage::OrderUpdate(sample_order_update_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(json["order_id"].is_string(), "order_id must be at root");
        assert!(json["client_order_id"].is_string());
        assert!(json["exchange"].is_string());
        assert!(json["pair"].is_string());
        assert!(json["status"].is_string());
        assert!(
            json["quantity"].is_string(),
            "quantity must be a string, never f64"
        );
        assert!(json["data"].is_null(), "no data wrapper key");
    }

    #[test]
    fn order_update_fill_price_is_none_when_order_open() {
        let payload = sample_order_update_payload();
        assert!(
            payload.fill_price.is_none(),
            "fill_price must be None for open orders"
        );
        let msg = WsMessage::OrderUpdate(payload);
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(json["fill_price"].is_null());
    }

    #[test]
    fn order_update_fill_price_is_string_when_filled() {
        let mut payload = sample_order_update_payload();
        payload.status = "filled".to_string();
        payload.fill_price = Some("58000".to_string());
        let msg = WsMessage::OrderUpdate(payload);
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["fill_price"], "58000");
        assert_eq!(json["status"], "filled");
    }

    #[test]
    fn order_update_quantity_is_string_not_number() {
        let msg = WsMessage::OrderUpdate(sample_order_update_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert!(
            json["quantity"].is_string(),
            "quantity must be serialized as a string to avoid float precision loss"
        );
    }

    #[test]
    fn order_update_round_trips_serialize_deserialize() {
        let original = WsMessage::OrderUpdate(sample_order_update_payload());
        let json = serde_json::to_string(&original).unwrap();
        let parsed: WsMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn price_consumer_ignores_order_update_type_without_error() {
        let json = r#"{"type":"order_update","order_id":"123","client_order_id":"abc","exchange":"tabdeal","pair":"USDT/IRT","market_type":"spot","side":"buy","status":"open","quantity":"100","fill_price":null,"strategy_id":null,"timestamp":"2026-06-20T00:00:00Z"}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        assert!(!matches!(msg, WsMessage::PriceUpdate(_)));
    }

    #[test]
    fn price_payload_from_price_maps_all_fields() {
        let price = sample_price();
        let payload = PricePayload::from(&price);
        assert_eq!(payload.exchange, "tabdeal");
        assert_eq!(payload.pair, "USDT/IRT");
        assert_eq!(payload.bid, 175_500);
        assert_eq!(payload.ask, 175_520);
    }
}
