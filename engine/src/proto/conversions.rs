//! Conversions from the JSON wire payloads to their protobuf equivalents, plus
//! `encode_*` helpers that produce the bytes published to Kafka.

use prost::Message;

use super::v1;
use crate::candle::entity::CandlePayload;
use crate::wire_message::SignalPayload;

impl From<&CandlePayload> for v1::Candle {
    fn from(c: &CandlePayload) -> Self {
        v1::Candle {
            exchange: c.exchange.clone(),
            pair: c.pair.clone(),
            interval: c.interval.clone(),
            time: c.time.timestamp_millis(),
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        }
    }
}

impl From<&SignalPayload> for v1::Signal {
    fn from(s: &SignalPayload) -> Self {
        v1::Signal {
            signal_id: s.signal_id.clone(),
            strategy_id: s.strategy_id.clone(),
            exchange: s.exchange.clone(),
            pair: s.pair.clone(),
            action: s.action.clone(),
            confidence: s.confidence,
            timestamp: s.timestamp.timestamp_millis(),
            stop_loss: s.stop_loss,
            take_profit: s.take_profit,
        }
    }
}

/// Serializes a closed candle to the protobuf bytes published to the Kafka
/// `candles` topic.
pub fn encode_candle(candle: &CandlePayload) -> Vec<u8> {
    v1::Candle::from(candle).encode_to_vec()
}

/// Serializes a signal to the protobuf bytes published to the Kafka `signals`
/// topic.
pub fn encode_signal(signal: &SignalPayload) -> Vec<u8> {
    v1::Signal::from(signal).encode_to_vec()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    use super::*;

    fn at_millis(ms: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(ms).expect("valid millis")
    }

    fn sample_candle() -> CandlePayload {
        CandlePayload {
            exchange: "coinex".to_string(),
            pair: "BTC/USDT".to_string(),
            interval: "1h".to_string(),
            time: at_millis(1_700_000_000_123),
            open: 3_074_000,
            high: 3_100_000,
            low: 3_050_000,
            close: 3_090_000,
            volume: 12_345,
        }
    }

    fn sample_signal() -> SignalPayload {
        SignalPayload {
            signal_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            strategy_id: "spread_threshold".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: "buy".to_string(),
            confidence: 0.873_456_789,
            timestamp: at_millis(1_700_000_000_123),
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn candle_proto_round_trips_serialize_deserialize() {
        let original = v1::Candle::from(&sample_candle());
        let bytes = original.encode_to_vec();
        let decoded = v1::Candle::decode(&bytes[..]).expect("decode candle");
        assert_eq!(original, decoded);
    }

    #[test]
    fn signal_proto_round_trips_serialize_deserialize() {
        let original = v1::Signal::from(&sample_signal());
        let bytes = original.encode_to_vec();
        let decoded = v1::Signal::decode(&bytes[..]).expect("decode signal");
        assert_eq!(original, decoded);
    }

    #[test]
    fn candle_proto_and_json_carry_identical_fields() {
        let payload = sample_candle();
        let proto = v1::Candle::from(&payload);
        let json: Value = serde_json::to_value(&payload).unwrap();

        assert_eq!(proto.exchange, json["exchange"].as_str().unwrap());
        assert_eq!(proto.pair, json["pair"].as_str().unwrap());
        assert_eq!(proto.interval, json["interval"].as_str().unwrap());
        assert_eq!(proto.open, json["open"].as_u64().unwrap());
        assert_eq!(proto.high, json["high"].as_u64().unwrap());
        assert_eq!(proto.low, json["low"].as_u64().unwrap());
        assert_eq!(proto.close, json["close"].as_u64().unwrap());
        assert_eq!(proto.volume, json["volume"].as_u64().unwrap());

        // The JSON path keeps an RFC3339 timestamp; the proto path keeps the
        // same instant as Unix millis. Both must denote the identical moment.
        let json_instant = DateTime::parse_from_rfc3339(json["time"].as_str().unwrap())
            .unwrap()
            .timestamp_millis();
        assert_eq!(proto.time, json_instant);
    }

    #[test]
    fn signal_proto_and_json_carry_identical_fields() {
        let mut payload = sample_signal();
        payload.stop_loss = Some(173_460);
        payload.take_profit = Some(184_080);
        let proto = v1::Signal::from(&payload);
        let json: Value = serde_json::to_value(&payload).unwrap();

        assert_eq!(proto.signal_id, json["signal_id"].as_str().unwrap());
        assert_eq!(proto.strategy_id, json["strategy_id"].as_str().unwrap());
        assert_eq!(proto.exchange, json["exchange"].as_str().unwrap());
        assert_eq!(proto.pair, json["pair"].as_str().unwrap());
        assert_eq!(proto.action, json["action"].as_str().unwrap());
        assert_eq!(proto.confidence, json["confidence"].as_f64().unwrap());
        assert_eq!(proto.stop_loss, Some(json["stop_loss"].as_u64().unwrap()));
        assert_eq!(
            proto.take_profit,
            Some(json["take_profit"].as_u64().unwrap())
        );
        let json_instant = DateTime::parse_from_rfc3339(json["timestamp"].as_str().unwrap())
            .unwrap()
            .timestamp_millis();
        assert_eq!(proto.timestamp, json_instant);
    }

    #[test]
    fn candle_proto_preserves_max_u64_prices_without_truncation() {
        let mut payload = sample_candle();
        payload.high = u64::MAX;
        payload.volume = u64::MAX;
        let decoded = v1::Candle::decode(&encode_candle(&payload)[..]).unwrap();
        assert_eq!(decoded.high, u64::MAX, "uint64 must not truncate to i64");
        assert_eq!(decoded.volume, u64::MAX);
    }

    #[test]
    fn candle_proto_round_trips_every_interval_string() {
        for interval in ["1m", "5m", "15m", "1h"] {
            let mut payload = sample_candle();
            payload.interval = interval.to_string();
            let decoded = v1::Candle::decode(&encode_candle(&payload)[..]).unwrap();
            assert_eq!(decoded.interval, interval);
        }
    }

    #[test]
    fn candle_proto_preserves_negative_pre_epoch_timestamp() {
        let mut payload = sample_candle();
        payload.time = at_millis(-86_400_000); // one day before the Unix epoch
        let decoded = v1::Candle::decode(&encode_candle(&payload)[..]).unwrap();
        assert_eq!(decoded.time, -86_400_000, "int64 time must stay signed");
    }

    #[test]
    fn signal_proto_round_trips_every_action_string() {
        for action in ["buy", "sell", "hold"] {
            let mut payload = sample_signal();
            payload.action = action.to_string();
            let decoded = v1::Signal::decode(&encode_signal(&payload)[..]).unwrap();
            assert_eq!(decoded.action, action);
        }
    }

    #[test]
    fn signal_proto_keeps_stop_loss_and_take_profit_absent_when_none() {
        let payload = sample_signal();
        assert!(payload.stop_loss.is_none() && payload.take_profit.is_none());
        let decoded = v1::Signal::decode(&encode_signal(&payload)[..]).unwrap();
        assert_eq!(
            decoded.stop_loss, None,
            "an unset stop_loss must decode as None, not 0"
        );
        assert_eq!(decoded.take_profit, None);
    }

    #[test]
    fn signal_proto_distinguishes_zero_from_absent_stop_loss() {
        let mut payload = sample_signal();
        payload.stop_loss = Some(0);
        let decoded = v1::Signal::decode(&encode_signal(&payload)[..]).unwrap();
        assert_eq!(
            decoded.stop_loss,
            Some(0),
            "an explicit zero stop must survive as Some(0), not collapse to None"
        );
    }

    #[test]
    fn signal_proto_preserves_confidence_precision() {
        let payload = sample_signal();
        let decoded = v1::Signal::decode(&encode_signal(&payload)[..]).unwrap();
        assert_eq!(
            decoded.confidence, payload.confidence,
            "double confidence must survive the round trip bit-for-bit"
        );
    }

    #[test]
    fn encode_candle_then_decode_recovers_all_fields() {
        let payload = sample_candle();
        let decoded = v1::Candle::decode(&encode_candle(&payload)[..]).unwrap();
        assert_eq!(decoded, v1::Candle::from(&payload));
    }
}
