//! The `Ticker` is the UI's view of one (exchange, pair) price stream —
//! the data behind a single ticker card. It's intentionally decoupled
//! from the wire [`crate::protocol::PriceMessage`] so the UI's shape can
//! evolve independently of the backend's JSON.

/// Whether the latest update moved the price up, down, or left it
/// unchanged. Drives the green/red flash on a ticker card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Unchanged,
}

/// Compares a previous and new bid to decide which way the card should
/// flash. Pure function: no signals, no rendering, trivially testable.
pub fn direction(previous_bid: f64, new_bid: f64) -> Direction {
    if new_bid > previous_bid {
        Direction::Up
    } else if new_bid < previous_bid {
        Direction::Down
    } else {
        Direction::Unchanged
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ticker {
    pub exchange: String,
    pub pair: String,
    pub bid: f64,
    pub ask: f64,
}

impl Ticker {
    pub fn new(exchange: impl Into<String>, pair: impl Into<String>, bid: f64, ask: f64) -> Self {
        Self {
            exchange: exchange.into(),
            pair: pair.into(),
            bid,
            ask,
        }
    }

    /// Stable identity for a ticker, used as the map key in app state and
    /// the DOM key when rendering the card list.
    pub fn key(&self) -> String {
        format!("{}:{}", self.exchange, self.pair)
    }

    pub fn spread(&self) -> f64 {
        self.ask - self.bid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_is_up_when_price_increases() {
        assert_eq!(direction(100.0, 101.0), Direction::Up);
    }

    #[test]
    fn direction_is_down_when_price_decreases() {
        assert_eq!(direction(100.0, 99.0), Direction::Down);
    }

    #[test]
    fn direction_is_unchanged_when_price_is_the_same() {
        assert_eq!(direction(100.0, 100.0), Direction::Unchanged);
    }

    #[test]
    fn ticker_key_combines_exchange_and_pair() {
        let t = Ticker::new("tabdeal", "USDT/IRT", 92815.0, 92936.0);
        assert_eq!(t.key(), "tabdeal:USDT/IRT");
    }

    #[test]
    fn ticker_spread_is_ask_minus_bid() {
        let t = Ticker::new("tabdeal", "USDT/IRT", 92815.0, 92936.0);
        assert_eq!(t.spread(), 121.0);
    }

    #[test]
    fn different_pairs_on_same_exchange_have_different_keys() {
        let a = Ticker::new("tabdeal", "USDT/IRT", 1.0, 2.0);
        let b = Ticker::new("tabdeal", "BTC/IRT", 1.0, 2.0);
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn same_pair_on_different_exchanges_have_different_keys() {
        let a = Ticker::new("tabdeal", "USDT/IRT", 1.0, 2.0);
        let b = Ticker::new("nobitex", "USDT/IRT", 1.0, 2.0);
        assert_ne!(a.key(), b.key());
    }
}
