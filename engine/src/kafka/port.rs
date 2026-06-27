use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PublisherError {
    #[error("publish failed: {0}")]
    PublishFailed(String),
}

/// Publishes serialized messages to an external message broker.
#[async_trait]
pub trait MessagePublisher: Send + Sync {
    /// Publishes a UTF-8 text `payload` to `topic` with the given `key` — used
    /// for the `prices` JSON topic. Errors are non-fatal: the caller logs
    /// them and continues broadcasting to WS clients.
    async fn publish(&self, topic: &str, key: &str, payload: &str) -> Result<(), PublisherError>;

    /// Publishes a binary `payload` to `topic` with the given `key` — used for
    /// the protobuf topics (`candles`/`signals`, ROADMAP Loop 4c). Same
    /// non-fatal error contract as [`publish`](Self::publish).
    async fn publish_bytes(
        &self,
        topic: &str,
        key: &str,
        payload: &[u8],
    ) -> Result<(), PublisherError>;
}

#[cfg(test)]
pub mod mock {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{MessagePublisher, PublisherError};

    pub struct MockPublisher {
        messages: Mutex<Vec<(String, String, String)>>,
        byte_messages: Mutex<Vec<(String, String, Vec<u8>)>>,
        should_fail: bool,
    }

    impl Default for MockPublisher {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockPublisher {
        pub fn new() -> Self {
            Self {
                messages: Mutex::new(vec![]),
                byte_messages: Mutex::new(vec![]),
                should_fail: false,
            }
        }

        pub fn failing() -> Self {
            Self {
                messages: Mutex::new(vec![]),
                byte_messages: Mutex::new(vec![]),
                should_fail: true,
            }
        }

        pub fn published(&self) -> Vec<(String, String, String)> {
            self.messages.lock().unwrap().clone()
        }

        pub fn published_bytes(&self) -> Vec<(String, String, Vec<u8>)> {
            self.byte_messages.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessagePublisher for MockPublisher {
        async fn publish(
            &self,
            topic: &str,
            key: &str,
            payload: &str,
        ) -> Result<(), PublisherError> {
            if self.should_fail {
                return Err(PublisherError::PublishFailed("forced failure".into()));
            }
            self.messages
                .lock()
                .unwrap()
                .push((topic.into(), key.into(), payload.into()));
            Ok(())
        }

        async fn publish_bytes(
            &self,
            topic: &str,
            key: &str,
            payload: &[u8],
        ) -> Result<(), PublisherError> {
            if self.should_fail {
                return Err(PublisherError::PublishFailed("forced failure".into()));
            }
            self.byte_messages
                .lock()
                .unwrap()
                .push((topic.into(), key.into(), payload.to_vec()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockPublisher;
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn publisher_records_topic_key_and_payload() {
        let publisher = MockPublisher::new();
        publisher
            .publish("prices", "tabdeal:USDT/IRT", r#"{"bid":175500}"#)
            .await
            .unwrap();

        let messages = publisher.published();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "prices");
        assert_eq!(messages[0].1, "tabdeal:USDT/IRT");
        assert_eq!(messages[0].2, r#"{"bid":175500}"#);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn publisher_records_multiple_messages_in_order() {
        let publisher = MockPublisher::new();
        publisher
            .publish("prices", "tabdeal:USDT/IRT", "msg1")
            .await
            .unwrap();
        publisher
            .publish("prices", "nobitex:BTC/IRT", "msg2")
            .await
            .unwrap();

        let messages = publisher.published();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].1, "tabdeal:USDT/IRT");
        assert_eq!(messages[1].1, "nobitex:BTC/IRT");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn publisher_returns_error_on_failure() {
        let publisher = MockPublisher::failing();
        let result = publisher.publish("prices", "key", "payload").await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn publisher_records_binary_payload_separately_from_text() {
        let publisher = MockPublisher::new();
        publisher
            .publish_bytes("candles.proto", "coinex:BTC/USDT", &[0x08, 0x96, 0x01])
            .await
            .unwrap();

        assert!(
            publisher.published().is_empty(),
            "binary publish must not land in the text channel"
        );
        let bytes = publisher.published_bytes();
        assert_eq!(bytes.len(), 1);
        assert_eq!(bytes[0].0, "candles.proto");
        assert_eq!(bytes[0].1, "coinex:BTC/USDT");
        assert_eq!(bytes[0].2, vec![0x08, 0x96, 0x01]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn publisher_returns_error_on_publish_bytes_failure() {
        let publisher = MockPublisher::failing();
        let result = publisher
            .publish_bytes("signals.proto", "key", &[1, 2, 3])
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn publisher_error_displays_message() {
        let err = PublisherError::PublishFailed("broker down".into());
        assert_eq!(err.to_string(), "publish failed: broker down");
    }
}
