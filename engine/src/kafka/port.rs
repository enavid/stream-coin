use std::fmt;

use async_trait::async_trait;

#[derive(Debug)]
pub enum PublisherError {
    PublishFailed(String),
}

impl fmt::Display for PublisherError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PublisherError::PublishFailed(msg) => write!(f, "publish failed: {}", msg),
        }
    }
}

#[async_trait]
pub trait MessagePublisher: Send + Sync {
    async fn publish(&self, topic: &str, key: &str, payload: &str) -> Result<(), PublisherError>;
}

#[cfg(test)]
pub mod mock {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{MessagePublisher, PublisherError};

    pub struct MockPublisher {
        messages: Mutex<Vec<(String, String, String)>>,
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
                should_fail: false,
            }
        }

        pub fn failing() -> Self {
            Self {
                messages: Mutex::new(vec![]),
                should_fail: true,
            }
        }

        pub fn published(&self) -> Vec<(String, String, String)> {
            self.messages.lock().unwrap().clone()
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
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockPublisher;
    use super::*;

    #[tokio::test]
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

    #[tokio::test]
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

    #[tokio::test]
    async fn publisher_returns_error_on_failure() {
        let publisher = MockPublisher::failing();
        let result = publisher.publish("prices", "key", "payload").await;
        assert!(result.is_err());
    }

    #[test]
    fn publisher_error_displays_message() {
        let err = PublisherError::PublishFailed("broker down".into());
        assert_eq!(err.to_string(), "publish failed: broker down");
    }
}
