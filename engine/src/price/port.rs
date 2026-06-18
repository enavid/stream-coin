use std::fmt;

use async_trait::async_trait;

use crate::price::entity::{Price, TradingPair};

#[derive(Debug)]
pub enum PriceFeedError {
    Unavailable(String),
    ParseError(String),
}

impl fmt::Display for PriceFeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PriceFeedError::Unavailable(msg) => write!(f, "feed unavailable: {}", msg),
            PriceFeedError::ParseError(msg) => write!(f, "parse error: {}", msg),
        }
    }
}

#[async_trait]
pub trait PriceFeed: Send + Sync {
    async fn latest_price(&self, pair: &TradingPair) -> Result<Price, PriceFeedError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::exchange::entity::ExchangeId;
    use crate::price::entity::{Price, TradingPair};

    struct MockPriceFeed {
        price: Price,
    }

    #[async_trait]
    impl PriceFeed for MockPriceFeed {
        async fn latest_price(&self, _pair: &TradingPair) -> Result<Price, PriceFeedError> {
            Ok(self.price.clone())
        }
    }

    #[tokio::test]
    async fn price_feed_returns_price_for_pair() {
        let feed = MockPriceFeed {
            price: Price {
                exchange: ExchangeId::new("tabdeal"),
                pair: TradingPair::new("USDT", "IRR"),
                ask: 63_000_000,
                bid: 62_500_000,
                timestamp: Utc::now(),
            },
        };

        let result = feed.latest_price(&TradingPair::new("USDT", "IRR")).await;

        assert!(result.is_ok());
        let price = result.unwrap();
        assert_eq!(price.exchange.to_string(), "tabdeal");
        assert_eq!(price.spread(), 500_000);
    }

    #[tokio::test]
    async fn price_feed_error_is_display() {
        let err = PriceFeedError::Unavailable("connection failed".to_string());
        assert!(!err.to_string().is_empty());
    }
}
