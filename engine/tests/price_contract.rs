use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use stream_coin::exchange::entity::ExchangeId;
use stream_coin::kafka::producer::KafkaProducer;
use stream_coin::price::entity::{Price, TradingPair};

fn sample_price() -> Price {
    Price {
        exchange: ExchangeId::new("tabdeal"),
        pair: TradingPair::new("USDT", "IRT"),
        bid: 175_500,
        ask: 175_520,
        timestamp: Utc::now(),
    }
}

fn payload_as_json() -> Value {
    let payload = KafkaProducer::price_to_payload(&sample_price()).unwrap();
    serde_json::from_str(&payload).unwrap()
}

#[test]
fn price_payload_exchange_is_string() {
    assert!(payload_as_json()["exchange"].is_string());
}

#[test]
fn price_payload_pair_is_base_slash_quote_string() {
    let json = payload_as_json();
    let pair = json["pair"].as_str().expect("pair must be a string");
    let slash_count = pair.chars().filter(|&c| c == '/').count();
    assert_eq!(slash_count, 1, "pair must contain exactly one '/'");
}

#[test]
fn price_payload_bid_is_u64_not_float() {
    assert!(
        payload_as_json()["bid"].is_u64(),
        "bid must serialize as integer, not float"
    );
}

#[test]
fn price_payload_ask_is_u64_not_float() {
    assert!(
        payload_as_json()["ask"].is_u64(),
        "ask must serialize as integer, not float"
    );
}

#[test]
fn price_payload_timestamp_is_rfc3339_string() {
    let json = payload_as_json();
    let ts = json["timestamp"]
        .as_str()
        .expect("timestamp must be a string");
    chrono::DateTime::parse_from_rfc3339(ts).expect("timestamp must be valid RFC 3339");
}

#[test]
fn price_payload_has_exactly_five_fields() {
    let json = payload_as_json();
    let field_count = json.as_object().unwrap().len();
    assert_eq!(
        field_count, 5,
        "payload must have exactly 5 fields (exchange, pair, bid, ask, timestamp); \
         got {field_count} — a field was added or removed without updating PriceMessage"
    );
}

#[test]
fn price_payload_field_names_match_price_message_struct() {
    let json = payload_as_json();
    let obj = json.as_object().unwrap();
    for key in ["exchange", "pair", "bid", "ask", "timestamp"] {
        assert!(obj.contains_key(key), "missing field: {key}");
    }
}

#[derive(Deserialize)]
struct PriceMessage {
    #[allow(dead_code)]
    exchange: String,
    #[allow(dead_code)]
    pair: String,
    #[allow(dead_code)]
    bid: u64,
    #[allow(dead_code)]
    ask: u64,
    #[allow(dead_code)]
    timestamp: String,
}

#[test]
fn price_payload_parses_as_price_message_without_error() {
    let payload = KafkaProducer::price_to_payload(&sample_price()).unwrap();
    serde_json::from_str::<PriceMessage>(&payload)
        .expect("engine payload must be parseable as PriceMessage");
}
