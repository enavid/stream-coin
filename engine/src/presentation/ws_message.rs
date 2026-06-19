use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::price::entity::Price;

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    PriceUpdate(PricePayload),
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
    fn price_payload_from_price_maps_all_fields() {
        let price = sample_price();
        let payload = PricePayload::from(&price);
        assert_eq!(payload.exchange, "tabdeal");
        assert_eq!(payload.pair, "USDT/IRT");
        assert_eq!(payload.bid, 175_500);
        assert_eq!(payload.ask, 175_520);
    }
}
