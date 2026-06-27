pub mod historical_adapter;
pub mod ws_adapter;

pub use historical_adapter::CoinexHistoricalAdapter;
pub use ws_adapter::CoinexWsAdapter;

use crate::price::entity::TradingPair;

/// Converts a CoinEx market name (e.g. `"BTCUSDT"`) to a canonical
/// [`TradingPair`] by stripping the known quote-currency suffix. Shared by
/// the WS adapter (depth pushes) and the historical REST adapter (klines) so
/// the two never drift on which suffixes are recognized.
///
/// Returns `None` for a market whose quote suffix we don't recognize (M4):
/// the previous behaviour fabricated a pair with an empty quote
/// (`TradingPair::new(market, "")`), which then flowed silently into Kafka and
/// the WS feed as a malformed pair. Callers must drop (and log) the `None`
/// rather than publish a fabricated pair. A `None` also guards against a base
/// that equals the whole market (e.g. the market string *is* `"USDT"`), which
/// would otherwise yield an empty base.
pub(crate) fn market_to_pair(market: &str) -> Option<TradingPair> {
    for quote in ["USDT", "USDC", "BTC"] {
        if let Some(base) = market.strip_suffix(quote) {
            if base.is_empty() {
                return None;
            }
            return Some(TradingPair::new(base, quote));
        }
    }
    None
}

/// Result of classifying an HTTP response status per the project's
/// transient/permanent error rule. `Success` covers 2xx (caller proceeds to
/// parse the body); shared by every CoinEx REST caller (historical klines,
/// top-market seeder) so they never disagree on the 4xx/5xx boundary.
pub(crate) enum HttpStatusClass {
    Success,
    Transient { status: u16, body: String },
    Permanent { status: u16, body: String },
}

pub(crate) fn classify_http_status(status: u16, body: &str) -> HttpStatusClass {
    match status {
        200..=299 => HttpStatusClass::Success,
        // 429 Too Many Requests is a rate-limit, not a permanent failure (M3):
        // classifying it permanent aborted the whole backfill on the first
        // throttle instead of backing off and retrying.
        429 | 500..=599 => HttpStatusClass::Transient {
            status,
            body: body.to_string(),
        },
        _ => HttpStatusClass::Permanent {
            status,
            body: body.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_to_pair_strips_known_quote_suffixes() {
        assert_eq!(
            market_to_pair("BTCUSDT"),
            Some(TradingPair::new("BTC", "USDT"))
        );
        assert_eq!(
            market_to_pair("ETHUSDC"),
            Some(TradingPair::new("ETH", "USDC"))
        );
        assert_eq!(
            market_to_pair("ETHBTC"),
            Some(TradingPair::new("ETH", "BTC"))
        );
    }

    #[test]
    fn unknown_quote_suffix_is_dropped_not_fabricated() {
        // Previously fabricated TradingPair { base: "DOGEIRR", quote: "" } and
        // published it; now an unrecognized quote yields None so the caller drops it.
        assert_eq!(market_to_pair("DOGEIRR"), None);
        assert_eq!(market_to_pair("RANDOMTHING"), None);
    }

    #[test]
    fn market_equal_to_quote_currency_is_dropped() {
        // The market string is the bare quote — stripping it leaves an empty
        // base, which must not become a pair with an empty base.
        assert_eq!(market_to_pair("USDT"), None);
        assert_eq!(market_to_pair("BTC"), None);
    }

    #[test]
    fn classify_http_status_429_is_transient() {
        // 429 Too Many Requests must back off and retry, not abort the backfill.
        match classify_http_status(429, "rate limited") {
            HttpStatusClass::Transient { status, .. } => assert_eq!(status, 429),
            other => panic!("429 must be Transient, got {:?}", DebugClass(&other)),
        }
    }

    #[test]
    fn classify_http_status_5xx_is_transient() {
        assert!(matches!(
            classify_http_status(503, "x"),
            HttpStatusClass::Transient { .. }
        ));
    }

    #[test]
    fn classify_http_status_other_4xx_is_permanent() {
        assert!(matches!(
            classify_http_status(404, "x"),
            HttpStatusClass::Permanent { .. }
        ));
    }

    #[test]
    fn classify_http_status_2xx_is_success() {
        assert!(matches!(
            classify_http_status(200, "{}"),
            HttpStatusClass::Success
        ));
    }

    // Small helper so the panic message in the 429 test can print the variant
    // without requiring `Debug` on `HttpStatusClass` itself.
    struct DebugClass<'a>(&'a HttpStatusClass);
    impl std::fmt::Debug for DebugClass<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self.0 {
                HttpStatusClass::Success => write!(f, "Success"),
                HttpStatusClass::Transient { status, .. } => write!(f, "Transient({status})"),
                HttpStatusClass::Permanent { status, .. } => write!(f, "Permanent({status})"),
            }
        }
    }
}
