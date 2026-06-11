use std::fmt;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingPair {
    pub base: String,
    pub quote: String,
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

#[derive(Debug, Clone)]
pub struct Price {
    pub exchange: String,
    pub pair: TradingPair,
    pub ask: u64,
    pub bid: u64,
    pub timestamp: DateTime<Utc>,
}

impl Price {
    pub fn spread(&self) -> u64 {
        self.ask - self.bid
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
    fn price_spread_is_ask_minus_bid() {
        let price = Price {
            exchange: "nobitex".to_string(),
            pair: TradingPair::new("USDT", "IRR"),
            ask: 63_000_000,
            bid: 62_500_000,
            timestamp: Utc::now(),
        };
        assert_eq!(price.spread(), 500_000);
    }
}
