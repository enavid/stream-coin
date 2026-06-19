use std::fmt;

use chrono::{DateTime, Utc};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

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

impl<'de> Deserialize<'de> for TradingPair {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let (base, quote) = s
            .split_once('/')
            .ok_or_else(|| de::Error::custom("symbol must be BASE/QUOTE format (e.g. USDT/IRT)"))?;
        if base.is_empty() || quote.is_empty() {
            return Err(de::Error::custom("base and quote must not be empty"));
        }
        if quote.contains('/') {
            return Err(de::Error::custom(
                "symbol must have exactly one '/' separator",
            ));
        }
        Ok(TradingPair::new(base, quote))
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
    /// Price in Iranian Rial (IRR). 1 unit = 1 IRR.
    /// Exchange adapters must normalize to this unit before constructing Price.
    pub ask: u64,
    /// Price in Iranian Rial (IRR). 1 unit = 1 IRR.
    /// Exchange adapters must normalize to this unit before constructing Price.
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

    // --- TradingPair Deserialize tests ---

    #[test]
    fn trading_pair_deserialize_accepts_base_slash_quote() {
        let pair: TradingPair = serde_json::from_str("\"USDT/IRT\"").unwrap();
        assert_eq!(pair.base, "USDT");
        assert_eq!(pair.quote, "IRT");
    }

    #[test]
    fn trading_pair_deserialize_rejects_no_slash() {
        let result: Result<TradingPair, _> = serde_json::from_str("\"USDTIRT\"");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("BASE/QUOTE"),
            "error must mention BASE/QUOTE format, got: {err}"
        );
    }

    #[test]
    fn trading_pair_deserialize_rejects_empty_base() {
        let result: Result<TradingPair, _> = serde_json::from_str("\"/IRT\"");
        assert!(result.is_err());
    }

    #[test]
    fn trading_pair_deserialize_rejects_empty_quote() {
        let result: Result<TradingPair, _> = serde_json::from_str("\"USDT/\"");
        assert!(result.is_err());
    }

    #[test]
    fn trading_pair_deserialize_rejects_multiple_slashes() {
        let result: Result<TradingPair, _> = serde_json::from_str("\"USDT/IRT/EXTRA\"");
        assert!(result.is_err());
    }

    #[test]
    fn trading_pair_deserialize_rejects_empty_string() {
        let result: Result<TradingPair, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn trading_pair_serialize_produces_base_slash_quote() {
        let pair = TradingPair::new("BTC", "IRT");
        let json = serde_json::to_string(&pair).unwrap();
        assert_eq!(json, "\"BTC/IRT\"");
    }

    #[test]
    fn trading_pair_round_trips_serialize_then_deserialize() {
        let original = TradingPair::new("USDT", "IRT");
        let json = serde_json::to_string(&original).unwrap();
        let parsed: TradingPair = serde_json::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

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
