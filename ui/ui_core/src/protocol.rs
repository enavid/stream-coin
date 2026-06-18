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
        serde_json::from_str(raw)
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
        r#"{"exchange":"tabdeal","pair":"USDT/IRT","ask":92936,"bid":92815,"timestamp":"2026-06-18T10:26:33.123Z"}"#
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
        let json = serde_json::to_string(&original).unwrap();
        let parsed = PriceMessage::parse(&json).unwrap();
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
}
