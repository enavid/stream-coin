use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

use crate::exchange::entity::ExchangeId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingPair {
    pub base: String,
    pub quote: String,
}

impl Serialize for TradingPair {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl TradingPair {
    pub fn new(base: &str, quote: &str) -> Self {
        Self {
            base: base.to_string(),
            quote: quote.to_string(),
        }
    }
}

impl fmt::Display for TradingPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.base, self.quote)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Price {
    pub exchange: ExchangeId,
    pub pair: TradingPair,
    pub ask: u64,
    pub bid: u64,
    pub timestamp: DateTime<Utc>,
}

impl Price {
    pub fn spread(&self) -> u64 {
        self.ask.saturating_sub(self.bid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn trading_pair_display_is_base_slash_quote() {
        let pair = TradingPair::new("USDT", "IRR");
        assert_eq!(pair.to_string(), "USDT/IRR");
    }

    #[test]
    fn price_exchange_is_exchange_id() {
        let price = Price {
            exchange: ExchangeId::new("Tabdeal"),
            pair: TradingPair::new("USDT", "IRR"),
            ask: 63_000_000,
            bid: 62_500_000,
            timestamp: Utc::now(),
        };
        assert_eq!(price.exchange.to_string(), "tabdeal");
    }

    #[test]
    fn price_spread_is_ask_minus_bid() {
        let price = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRR"),
            ask: 63_000_000,
            bid: 62_500_000,
            timestamp: Utc::now(),
        };
        assert_eq!(price.spread(), 500_000);
    }

    #[test]
    fn spread_inverted_quote_does_not_panic() {
        let price = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRR"),
            ask: 100,
            bid: 200,
            timestamp: Utc::now(),
        };
        assert_eq!(price.spread(), 0);
    }

    #[test]
    fn spread_equal_bid_ask_is_zero() {
        let price = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRR"),
            ask: 500,
            bid: 500,
            timestamp: Utc::now(),
        };
        assert_eq!(price.spread(), 0);
    }

    #[test]
    fn spread_max_u64_values_does_not_overflow() {
        let price = Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRR"),
            ask: u64::MAX,
            bid: u64::MAX,
            timestamp: Utc::now(),
        };
        assert_eq!(price.spread(), 0);
    }
}
