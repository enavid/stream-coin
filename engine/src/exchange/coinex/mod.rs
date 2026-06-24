pub mod historical_adapter;
pub mod ws_adapter;

pub use historical_adapter::CoinexHistoricalAdapter;
pub use ws_adapter::CoinexWsAdapter;

use crate::price::entity::TradingPair;

/// Converts a CoinEx market name (e.g. `"BTCUSDT"`) to a canonical
/// [`TradingPair`] by stripping the known quote-currency suffix. Shared by
/// the WS adapter (depth pushes) and the historical REST adapter (klines) so
/// the two never drift on which suffixes are recognized.
pub(crate) fn market_to_pair(market: &str) -> TradingPair {
    if let Some(base) = market.strip_suffix("USDT") {
        TradingPair::new(base, "USDT")
    } else if let Some(base) = market.strip_suffix("USDC") {
        TradingPair::new(base, "USDC")
    } else if let Some(base) = market.strip_suffix("BTC") {
        TradingPair::new(base, "BTC")
    } else {
        TradingPair::new(market, "")
    }
}

/// Truncates a CoinEx decimal-string field (price, volume, value — all
/// returned as JSON strings) to a non-negative minor-unit `u64`. Per the
/// project's Financial Precision rule: never parsed as `f64`. Shared by the
/// historical kline adapter and the top-market seeder so both exchange-data
/// parsers agree on the same truncation behavior.
pub(crate) fn parse_minor_units(s: &str) -> Result<u64, String> {
    if s.starts_with('-') {
        return Err(format!("value must be non-negative: {s}"));
    }
    let integer_part = s.split_once('.').map_or(s, |(int, _)| int);
    integer_part
        .parse::<u64>()
        .map_err(|_| format!("invalid numeric value: {s}"))
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
        500..=599 => HttpStatusClass::Transient {
            status,
            body: body.to_string(),
        },
        _ => HttpStatusClass::Permanent {
            status,
            body: body.to_string(),
        },
    }
}
