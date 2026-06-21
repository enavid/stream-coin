//! Pure, signal-free state transitions. Kept separate from
//! [`super::AppState`] so the update logic (insert ticker, cap the feed,
//! compute flash direction) is unit testable without a Dioxus runtime.

use std::collections::HashMap;

use crate::api::OrderItem;
use crate::domain::{direction, extract_time, Direction, Ticker};
use crate::protocol::{OrderUpdateMessage, PriceMessage, SignalMessage};

#[derive(Debug, Clone, PartialEq)]
pub struct FeedRow {
    pub key: String,
    pub time: String,
    pub exchange: String,
    pub pair: String,
    pub bid: f64,
    pub ask: f64,
}

impl FeedRow {
    fn from_message(msg: &PriceMessage, ticker: &Ticker) -> Self {
        Self {
            key: ticker.key(),
            time: extract_time(&msg.timestamp).to_string(),
            exchange: ticker.exchange.clone(),
            pair: ticker.pair.clone(),
            bid: ticker.bid,
            ask: ticker.ask,
        }
    }

    /// See [`Ticker::quote_currency`] — same `BASE/QUOTE` convention, just
    /// read off the feed row's own `pair` instead of a live `Ticker`.
    pub fn quote_currency(&self) -> &str {
        self.pair.split_once('/').map_or(&self.pair, |(_, q)| q)
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
    /// flash direction relative to its previous bid, and upserts its feed
    /// row in place — one row per (exchange, pair), never a new row per
    /// tick, so the live feed shows current state rather than a scrolling
    /// transaction log.
    pub fn apply(&mut self, msg: &PriceMessage) {
        let ticker: Ticker = msg.into();
        let key = ticker.key();

        if let Some(previous) = self.tickers.get(&key) {
            self.flash
                .insert(key.clone(), direction(previous.bid, ticker.bid));
        }

        let row = FeedRow::from_message(msg, &ticker);
        match self.feed.iter_mut().find(|r| r.key == key) {
            Some(existing) => *existing = row,
            None => self.feed.push(row),
        }

        self.tickers.insert(key, ticker);
    }

    pub fn remove(&mut self, key: &str) {
        self.tickers.remove(key);
        self.flash.remove(key);
        self.feed.retain(|row| row.key != key);
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

    /// Ends a ticker card's up/down highlight. The flash is meant to mark
    /// *this* tick, not become a permanent border color — without an
    /// explicit clear it never resets, since `apply()` only ever inserts
    /// or overwrites a flash, never removes one. The platform layer calls
    /// this after a short delay (see `ui/web/src/ws.rs`); kept here as a
    /// pure, timer-free mutation so it's unit testable.
    pub fn clear_flash(&mut self, key: &str) {
        self.flash.remove(key);
    }
}

/// Strategy signals are a true append log — `signal_id` is unique per
/// emission and never repeats, so unlike the ticker feed there is nothing
/// to update in place.
pub const MAX_SIGNAL_ROWS: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct SignalRow {
    pub signal_id: String,
    pub time: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
}

impl From<&SignalMessage> for SignalRow {
    fn from(msg: &SignalMessage) -> Self {
        Self {
            signal_id: msg.signal_id.clone(),
            time: extract_time(&msg.timestamp).to_string(),
            strategy_id: msg.strategy_id.clone(),
            exchange: msg.exchange.clone(),
            pair: msg.pair.clone(),
            action: msg.action.clone(),
            confidence: msg.confidence,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SignalStore {
    rows: Vec<SignalRow>,
}

impl SignalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, msg: &SignalMessage) {
        self.rows.insert(0, SignalRow::from(msg));
        self.rows.truncate(MAX_SIGNAL_ROWS);
    }

    pub fn rows(&self) -> &[SignalRow] {
        &self.rows
    }
}

/// Order updates for the same order arrive multiple times as it
/// transitions (`open` → `filled`) — keyed by `client_order_id` so a
/// later update replaces its row in place rather than duplicating it.
/// New orders are inserted at the front (most recent first), matching
/// an order-history view; in-place updates do not move the row.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRow {
    pub client_order_id: String,
    pub time: String,
    pub order_id: String,
    pub exchange: String,
    pub pair: String,
    pub side: String,
    pub status: String,
    pub quantity: String,
    pub fill_price: Option<String>,
    pub strategy_id: Option<String>,
}

impl From<&OrderUpdateMessage> for OrderRow {
    fn from(msg: &OrderUpdateMessage) -> Self {
        Self {
            client_order_id: msg.client_order_id.clone(),
            time: extract_time(&msg.timestamp).to_string(),
            order_id: msg.order_id.clone(),
            exchange: msg.exchange.clone(),
            pair: msg.pair.clone(),
            side: msg.side.clone(),
            status: msg.status.clone(),
            quantity: msg.quantity.clone(),
            fill_price: msg.fill_price.clone(),
            strategy_id: msg.strategy_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OrderStore {
    rows: Vec<OrderRow>,
}

impl OrderStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, msg: &OrderUpdateMessage) {
        match self
            .rows
            .iter_mut()
            .find(|r| r.client_order_id == msg.client_order_id)
        {
            Some(existing) => *existing = OrderRow::from(msg),
            None => self.rows.insert(0, OrderRow::from(msg)),
        }
    }

    pub fn rows(&self) -> &[OrderRow] {
        &self.rows
    }

    /// Seeds the table from `GET /v1/orders` on page load — the WS feed
    /// only carries *future* updates, so existing order history must be
    /// fetched separately. Replaces whatever rows were already present.
    pub fn seed(&mut self, items: &[OrderItem]) {
        self.rows = items.iter().map(OrderRow::from).collect();
    }
}

impl From<&OrderItem> for OrderRow {
    fn from(item: &OrderItem) -> Self {
        Self {
            client_order_id: item.client_order_id.clone(),
            time: extract_time(&item.created_at).to_string(),
            order_id: item.exchange_order_id.clone().unwrap_or_default(),
            exchange: item.exchange.clone(),
            pair: item.pair.clone(),
            side: item.side.clone(),
            status: item.status.clone(),
            quantity: item.quantity.clone(),
            fill_price: item.price.clone(),
            strategy_id: item.strategy_id.clone(),
        }
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
    fn feed_row_quote_currency_returns_the_part_after_the_slash() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        assert_eq!(store.feed()[0].quote_currency(), "IRT");
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
    fn apply_updates_the_existing_feed_row_in_place_for_a_repeated_ticker() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        // still exactly one row for this ticker, not a new row per tick
        assert_eq!(store.feed().len(), 1);
        assert_eq!(store.feed()[0].bid, 93000.0);
        assert_eq!(store.feed()[0].ask, 93100.0);
    }

    #[test]
    fn apply_inserts_one_feed_row_per_distinct_ticker() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("nobitex", "USDT/IRT", 92740, 92860));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        assert_eq!(store.feed().len(), 2);
    }

    #[test]
    fn apply_keeps_a_repeated_ticker_row_at_its_original_position() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("nobitex", "USDT/IRT", 92740, 92860));
        // re-apply the first ticker — it must not jump to the front
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));

        assert_eq!(store.feed()[0].exchange, "tabdeal");
        assert_eq!(store.feed()[0].bid, 93000.0);
        assert_eq!(store.feed()[1].exchange, "nobitex");
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
    fn clear_flash_removes_the_flash_for_a_key() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));
        store.apply(&msg("tabdeal", "USDT/IRT", 93000, 93100));
        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), Some(Direction::Up));

        store.clear_flash("tabdeal:USDT/IRT");

        assert_eq!(store.flash_for("tabdeal:USDT/IRT"), None);
    }

    #[test]
    fn clear_flash_on_unknown_key_is_a_no_op() {
        let mut store = TickerStore::new();
        store.clear_flash("does:not-exist");
        assert_eq!(store.flash_for("does:not-exist"), None);
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

    #[test]
    fn remove_drops_the_tickers_feed_row_too() {
        let mut store = TickerStore::new();
        store.apply(&msg("tabdeal", "USDT/IRT", 92815, 92936));

        store.remove("tabdeal:USDT/IRT");

        assert!(
            store.feed().is_empty(),
            "a stopped ticker must not leave a stale row in the live feed"
        );
    }

    // --- SignalStore tests ---

    fn signal(id: &str, strategy_id: &str) -> SignalMessage {
        SignalMessage {
            signal_id: id.to_string(),
            strategy_id: strategy_id.to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            action: "buy".to_string(),
            confidence: 0.9,
            timestamp: "2026-06-21T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn signal_store_prepends_new_signals() {
        let mut store = SignalStore::new();
        store.apply(&signal("sig-1", "spread_threshold"));
        store.apply(&signal("sig-2", "price_delta"));

        assert_eq!(store.rows()[0].signal_id, "sig-2");
        assert_eq!(store.rows()[1].signal_id, "sig-1");
    }

    #[test]
    fn signal_store_caps_at_max_rows() {
        let mut store = SignalStore::new();
        for i in 0..(MAX_SIGNAL_ROWS + 5) {
            store.apply(&signal(&format!("sig-{i}"), "spread_threshold"));
        }

        assert_eq!(store.rows().len(), MAX_SIGNAL_ROWS);
        assert_eq!(
            store.rows()[0].signal_id,
            format!("sig-{}", MAX_SIGNAL_ROWS + 4)
        );
    }

    // --- OrderStore tests ---

    fn order(client_order_id: &str, status: &str) -> OrderUpdateMessage {
        OrderUpdateMessage {
            order_id: "ord-1".to_string(),
            client_order_id: client_order_id.to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            market_type: "spot".to_string(),
            side: "buy".to_string(),
            status: status.to_string(),
            quantity: "100".to_string(),
            fill_price: None,
            strategy_id: None,
            timestamp: "2026-06-21T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn order_store_inserts_new_row_for_unseen_client_order_id() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "open"));
        store.apply(&order("cli-2", "open"));

        assert_eq!(store.rows().len(), 2);
        // most recent first
        assert_eq!(store.rows()[0].client_order_id, "cli-2");
    }

    #[test]
    fn order_store_updates_existing_row_by_client_order_id() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "open"));
        store.apply(&order("cli-2", "open"));
        store.apply(&order("cli-1", "filled"));

        assert_eq!(
            store.rows().len(),
            2,
            "updating an existing order must not duplicate its row"
        );
        let updated = store
            .rows()
            .iter()
            .find(|r| r.client_order_id == "cli-1")
            .unwrap();
        assert_eq!(updated.status, "filled");
    }

    #[test]
    fn order_store_seed_builds_rows_from_order_items() {
        let item = OrderItem {
            id: 1,
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            side: "buy".to_string(),
            order_type: "market".to_string(),
            quantity: "50".to_string(),
            price: None,
            status: "filled".to_string(),
            exchange_order_id: Some("ex-ord-9".to_string()),
            client_order_id: "cli-9".to_string(),
            strategy_id: None,
            created_at: "2026-06-21T10:00:00Z".to_string(),
        };

        let mut store = OrderStore::new();
        store.seed(&[item]);

        assert_eq!(store.rows().len(), 1);
        assert_eq!(store.rows()[0].client_order_id, "cli-9");
        assert_eq!(store.rows()[0].order_id, "ex-ord-9");
        assert_eq!(store.rows()[0].status, "filled");
    }

    #[test]
    fn order_store_seed_replaces_existing_rows() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "open"));

        store.seed(&[]);

        assert!(store.rows().is_empty());
    }

    #[test]
    fn order_store_update_does_not_move_the_rows_position() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "open"));
        store.apply(&order("cli-2", "open"));
        store.apply(&order("cli-1", "filled"));

        assert_eq!(
            store.rows()[0].client_order_id,
            "cli-2",
            "updating cli-1 in place must not move it back to the front"
        );
        assert_eq!(store.rows()[1].client_order_id, "cli-1");
    }
}
