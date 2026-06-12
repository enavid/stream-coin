use std::time::Duration;

use async_trait::async_trait;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};

use crate::kafka::port::{MessagePublisher, PublisherError};
use crate::price::entity::Price;

pub struct KafkaProducer {
    inner: FutureProducer,
}

impl KafkaProducer {
    pub fn new(brokers: &str) -> Result<Self, PublisherError> {
        let producer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .set("queue.buffering.max.ms", "100")
            .create::<FutureProducer>()
            .map_err(|e| PublisherError::PublishFailed(e.to_string()))?;

        Ok(Self { inner: producer })
    }

    pub fn price_to_payload(price: &Price) -> Result<String, serde_json::Error> {
        serde_json::to_string(price)
    }

    pub fn price_to_key(price: &Price) -> String {
        format!("{}:{}", price.exchange, price.pair)
    }
}

#[async_trait]
impl MessagePublisher for KafkaProducer {
    async fn publish(&self, topic: &str, key: &str, payload: &str) -> Result<(), PublisherError> {
        self.inner
            .send(
                FutureRecord::to(topic).key(key).payload(payload),
                Duration::from_secs(5),
            )
            .await
            .map(|_| ())
            .map_err(|(e, _)| PublisherError::PublishFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::exchange::entity::ExchangeId;
    use crate::price::entity::TradingPair;

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
    fn price_to_key_format_is_exchange_colon_pair() {
        let price = sample_price();
        assert_eq!(KafkaProducer::price_to_key(&price), "tabdeal:USDT/IRT");
    }

    #[test]
    fn price_to_payload_is_valid_json() {
        let price = sample_price();
        let payload = KafkaProducer::price_to_payload(&price).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(parsed.is_object());
    }

    #[test]
    fn price_to_payload_contains_exchange_as_string() {
        let price = sample_price();
        let payload = KafkaProducer::price_to_payload(&price).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["exchange"], "tabdeal");
    }

    #[test]
    fn price_to_payload_contains_pair_as_string() {
        let price = sample_price();
        let payload = KafkaProducer::price_to_payload(&price).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["pair"], "USDT/IRT");
    }

    #[test]
    fn price_to_payload_contains_bid_and_ask() {
        let price = sample_price();
        let payload = KafkaProducer::price_to_payload(&price).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["bid"], 175_500u64);
        assert_eq!(parsed["ask"], 175_520u64);
    }

    #[test]
    fn price_to_payload_contains_timestamp() {
        let price = sample_price();
        let payload = KafkaProducer::price_to_payload(&price).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(parsed["timestamp"].is_string());
    }

    #[test]
    fn kafka_producer_new_succeeds_with_broker_address() {
        let result = KafkaProducer::new("localhost:9092");
        assert!(result.is_ok());
    }
}
