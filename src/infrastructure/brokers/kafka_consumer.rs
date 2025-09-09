use std::sync::Arc;
use rdkafka::Message;
use futures::StreamExt;
use rdkafka::error::KafkaError;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};

pub async fn start_kafka_consumer<H>(
    broker_url: &str,
    topic: &str,
    group_id: &str,
    handler: Arc<H>,
) -> Result<(), KafkaError>
where
    H: Fn(String) + Send + Sync + 'static,
{
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", broker_url)
        .set("group.id", group_id)
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", "6000")
        .set("enable.auto.commit", "true")
        .create()?;

    consumer.subscribe(&[topic])?;

    println!("Kafka consumer connected to topic `{}`", topic);

    let mut stream = consumer.stream();

    while let Some(result) = stream.next().await {
        match result {
            Ok(message) => {
                if let Some(Ok(payload))= message.payload_view::<str>() {
                    handler(payload.to_string());
                }
            }
            Err(err) => {
                eprintln!("Kafka consume error: {:?}", err);
            }
        }
    }

    Ok(())
}
