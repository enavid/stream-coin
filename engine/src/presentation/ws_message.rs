use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::candle::entity::CandlePayload;
use crate::price::entity::Price;
use crate::strategy::entity::Signal;

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    PriceUpdate(PricePayload),
    Candle(CandlePayload),
    Signal(SignalPayload),
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct SignalPayload {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct PricePayload {
    pub exchange: String,
    pub pair: String,
    pub bid: u64,
    pub ask: u64,
    pub timestamp: DateTime<Utc>,
}

impl From<&Signal> for SignalPayload {
    fn from(s: &Signal) -> Self {
        SignalPayload {
            strategy_id: s.strategy_id.clone(),
            exchange: s.exchange.clone(),
            pair: s.pair.clone(),
            action: s.action.as_str().to_string(),
            confidence: s.confidence,
            timestamp: s.timestamp,
        }
    }
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
        use crate::candle::entity::CandlePayload;

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

    fn sample_signal_payload() -> SignalPayload {
        SignalPayload {
            strategy_id: "spread_threshold".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: "buy".to_string(),
            confidence: 0.85,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn signal_serializes_with_type_signal() {
        let msg = WsMessage::Signal(sample_signal_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "signal");
    }

    #[test]
    fn signal_fields_at_root_not_under_data_key() {
        let msg = WsMessage::Signal(sample_signal_payload());
        let json: Value = serde_json::to_value(&msg).unwrap();
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
    fn price_consumer_ignores_signal_type_without_error() {
        let json = r#"{"type":"signal","strategy_id":"s1","exchange":"tabdeal","pair":"USDT/IRT","action":"buy","confidence":0.9,"timestamp":"2026-06-20T00:00:00Z"}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        let is_price = matches!(msg, WsMessage::PriceUpdate(_));
        assert!(
            !is_price,
            "a signal message must not match the price_update variant"
        );
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
