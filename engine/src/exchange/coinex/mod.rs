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
