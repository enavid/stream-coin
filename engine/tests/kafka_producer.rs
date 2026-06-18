use std::time::Duration;

use chrono::Utc;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;

use stream_coin::exchange::entity::ExchangeId;
use stream_coin::kafka::port::MessagePublisher;
use stream_coin::kafka::producer::KafkaProducer;
use stream_coin::price::entity::{Price, TradingPair};

const BROKER: &str = "localhost:9092";
const TEST_TOPIC: &str = "prices-integration-test";

fn sample_price() -> Price {
    Price {
        exchange: ExchangeId::new("tabdeal"),
        pair: TradingPair::new("USDT", "IRT"),
        bid: 175_500,
        ask: 175_520,
        timestamp: Utc::now(),
    }
}

fn build_consumer(group_id: &str) -> StreamConsumer {
    ClientConfig::new()
        .set("bootstrap.servers", BROKER)
        .set("group.id", group_id)
        .set("auto.offset.reset", "earliest")
        .set("enable.auto.commit", "false")
        .set("session.timeout.ms", "6000")
        .create()
        .expect("consumer creation failed")
}

#[tokio::test]
async fn kafka_producer_publishes_price_without_error() {
    let producer = KafkaProducer::new(BROKER).expect("producer creation failed");
    let price = sample_price();

    let payload = KafkaProducer::price_to_payload(&price).unwrap();
    let key = KafkaProducer::price_to_key(&price);

    let result = producer.publish(TEST_TOPIC, &key, &payload).await;

    assert!(result.is_ok(), "publish returned error: {:?}", result.err());
}

#[tokio::test]
async fn kafka_producer_published_message_is_consumable() {
    let unique_group = format!(
        "test-group-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );

    let consumer: StreamConsumer = build_consumer(&unique_group);
    consumer
        .subscribe(&[TEST_TOPIC])
        .expect("subscription failed");

    let producer = KafkaProducer::new(BROKER).expect("producer creation failed");
    let price = sample_price();
    let payload = KafkaProducer::price_to_payload(&price).unwrap();
    let key = KafkaProducer::price_to_key(&price);

    producer
        .publish(TEST_TOPIC, &key, &payload)
        .await
        .expect("publish failed");

    let message = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            use futures_util::StreamExt;
            if let Some(Ok(msg)) = consumer.stream().next().await {
                return msg;
            }
        }
    })
    .await
    .expect("timed out waiting for message");

    let received_key = message
        .key()
        .and_then(|k| std::str::from_utf8(k).ok())
        .unwrap_or("");
    let received_payload = message
        .payload()
        .and_then(|p| std::str::from_utf8(p).ok())
        .unwrap_or("");

    assert_eq!(received_key, "tabdeal:USDT/IRT");

    let parsed: serde_json::Value = serde_json::from_str(received_payload).unwrap();
    assert_eq!(parsed["exchange"], "tabdeal");
    assert_eq!(parsed["pair"], "USDT/IRT");
    assert_eq!(parsed["bid"], 175_500u64);
    assert_eq!(parsed["ask"], 175_520u64);
}
