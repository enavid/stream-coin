//! Wire format shared with the backend's `GET /v1/ws` broadcast feed.
//!
//! This mirrors `stream_coin::price::entity::Price`'s `Serialize` output
//! field-for-field. It's intentionally a small, hand-maintained mirror
//! rather than a shared crate dependency: the backend can't compile to
//! `wasm32` (it links `rdkafka`/`redis` native libs), so the contract is
//! kept here and locked in by the round-trip tests below.

use serde::{Deserialize, Serialize};

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
}

impl From<&PriceMessage> for Ticker {
    fn from(msg: &PriceMessage) -> Self {
        Ticker::new(&msg.exchange, &msg.pair, msg.bid as f64, msg.ask as f64)
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

    #[test]
    fn price_message_parse_rejects_missing_type_field() {
        let json = r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#;
        assert!(
            PriceMessage::parse(json).is_err(),
            "payload without type field must be rejected"
        );
    }
}
