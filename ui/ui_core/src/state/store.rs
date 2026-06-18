//! Pure, signal-free state transitions. Kept separate from
//! [`super::AppState`] so the update logic (insert ticker, cap the feed,
//! compute flash direction) is unit testable without a Dioxus runtime.

use std::collections::HashMap;

use crate::domain::{direction, extract_time, Direction, Ticker};
use crate::protocol::PriceMessage;

/// Maximum rows kept in the live feed — older rows are dropped, matching
/// the "keep it simple" feed design (see ui/mockup.html).
pub const MAX_FEED_ROWS: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct FeedRow {
    pub time: String,
    pub exchange: String,
    pub pair: String,
    pub bid: f64,
    pub ask: f64,
}

impl FeedRow {
    fn from_message(msg: &PriceMessage, ticker: &Ticker) -> Self {
        Self {
            time: extract_time(&msg.timestamp).to_string(),
            exchange: ticker.exchange.clone(),
            pair: ticker.pair.clone(),
            bid: ticker.bid,
            ask: ticker.ask,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TickerStore {
    tickers: HashMap<String, Ticker>,
    feed: Vec<FeedRow>,
    flash: HashMap<String, Direction>,
}

impl TickerStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies an incoming price message: updates the ticker, records the
    /// flash direction relative to its previous bid, and prepends a feed
    /// row (capped at [`MAX_FEED_ROWS`]).
    pub fn apply(&mut self, msg: &PriceMessage) {
        let ticker: Ticker = msg.into();
        let key = ticker.key();

        if let Some(previous) = self.tickers.get(&key) {
            self.flash
                .insert(key.clone(), direction(previous.bid, ticker.bid));
        }

        self.feed.insert(0, FeedRow::from_message(msg, &ticker));
        self.feed.truncate(MAX_FEED_ROWS);

        self.tickers.insert(key, ticker);
    }

    pub fn remove(&mut self, key: &str) {
        self.tickers.remove(key);
        self.flash.remove(key);
    }

    pub fn tickers(&self) -> &HashMap<String, Ticker> {
        &self.tickers
    }

    pub fn feed(&self) -> &[FeedRow] {
        &self.feed
    }

    pub fn flash_for(&self, key: &str) -> Option<Direction> {
        self.flash.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(exchange: &str, pair: &str, bid: u64, ask: u64) -> PriceMessage {
        PriceMessage {
            exchange: exchange.to_string(),
            pair: pair.to_string(),
            ask,
            bid,
            timestamp: "2026-06-18T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn apply_inserts_a_new_ticker() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));

        let ticker = store.tickers().get("tabdeal:USDT/IRT").unwrap();
        assert_eq!(ticker.bid, 92815.0);
        assert_eq!(ticker.ask, 92936.0);
    }

    #[test]
    fn apply_overwrites_an_existing_ticker_with_the_same_key() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        assert_eq!(store.tickers().len(), 1);
        assert_eq!(store.tickers()["tabdeal:USDT/IRT"].bid, 93000.0);
    }

    #[test]
    fn apply_keeps_distinct_tickers_for_distinct_keys() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("nobitex", "USDT/IRT", 92740, 92860));

        assert_eq!(store.tickers().len(), 2);
    }

    #[test]
    fn apply_prepends_a_feed_row_for_every_message() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        assert_eq!(store.feed().len(), 2);
        // most recent first
        assert_eq!(store.feed()[0].bid, 93000.0);
        assert_eq!(store.feed()[1].bid, 92815.0);
    }

    #[test]
    fn apply_caps_the_feed_at_max_rows() {
        let mut store = TickerStore::new();
        for i in 0..(MAX_FEED_ROWS + 5) {
            store.apply(&msg(
                "tabdeal",
                "USDT/IRT",
                90000 + i as u64,
                90100 + i as u64,
            ));
        }

        assert_eq!(store.feed().len(), MAX_FEED_ROWS);
        // the most recent message is still at the front
        assert_eq!(store.feed()[0].bid, 90000.0 + (MAX_FEED_ROWS + 4) as f64);
    }

    #[test]
    fn apply_records_no_flash_direction_for_a_brand_new_ticker() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));

        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), None);
    }

    #[test]
    fn apply_records_up_when_bid_increases() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), Some(Direction::Up));
    }

    #[test]
    fn apply_records_down_when_bid_decreases() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 92000, 92100));

        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), Some(Direction::Down));
    }

    #[test]
    fn remove_drops_the_ticker_and_its_flash_state() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        store.remove("tabdeal:USDT/IRT");

        assert!(store.tickers().is_empty());
        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), None);
    }

    #[test]
    fn remove_on_unknown_key_is_a_no_op() {
        let mut store = TickerStore::new();
        store.remove("does:not-exist");
        assert!(store.tickers().is_empty());
    }
}
