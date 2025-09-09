use std::time::Duration;
use rdkafka::config::ClientConfig;
use rdkafka::producer::{FutureProducer, FutureRecord};


pub fn establish_kafka_producer(broker_url: &str) -> Result<FutureProducer, rdkafka::error::KafkaError> {
    ClientConfig::new()
        .set("bootstrap.servers", broker_url)
        .set("message.timeout.ms", "5000")
        .set("message.timeout.ms", "3000")
        .set("queue.buffering.max.messages", "100000")
        .set("queue.buffering.max.kbytes", "1048576")
        .create()
}

pub async fn send_to_kafka(
    producer: &FutureProducer,
    topic: &str,
    key: &str,
    payload: &str,
) -> Result<(), rdkafka::error::KafkaError> {
    let record = FutureRecord::to(topic)
        .key(key)
        .payload(payload);

    producer
        .send(record, Duration::from_secs(0))
        .await
        .map(|_| ())
        .map_err(|(e, _)| e)
}
