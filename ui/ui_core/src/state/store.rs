//! Pure, signal-free state transitions. Kept separate from
//! [`super::AppState`] so the update logic (insert ticker, cap the feed,
//! compute flash direction) is unit testable without a Dioxus runtime.

use std::collections::HashMap;

use crate::api::{BacktestResult, CandleItem, OrderItem};
use crate::domain::{direction, extract_time, Direction, Ticker};
use crate::protocol::{CandleMessage, OrderUpdateMessage, PriceMessage, SignalMessage};

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
    /// Full RFC3339 timestamp — `time` above is just the `HH:MM:SS` display
    /// slice (see `extract_time`), which isn't enough to place a marker at
    /// the right point on the chart's time axis.
    pub full_timestamp: String,
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
            full_timestamp: msg.timestamp.clone(),
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

/// The one thing Stage 10's live-order overlay can render today: an entry
/// marker at the open position's fill time/side. A live trade rectangle or
/// SL/TP lines would need `OrderUpdatePayload` to carry those prices, which
/// it doesn't — see `ROADMAP.md` Phase 7's "Stage 10" note.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OpenPositionMarker {
    pub time: String,
    pub side: String,
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

    /// The current open position for an exchange/pair, if any — the most
    /// recent row that isn't cancelled/failed. Intentionally simple (no
    /// fill-netting): Stage 10 only needs the one order to put an entry
    /// marker on, not a full position-tracking model.
    pub fn open_position_for(&self, exchange: &str, pair: &str) -> Option<OpenPositionMarker> {
        self.rows
            .iter()
            .find(|r| {
                r.exchange == exchange
                    && r.pair == pair
                    && !matches!(r.status.as_str(), "cancelled" | "failed")
            })
            .map(|r| OpenPositionMarker {
                time: r.full_timestamp.clone(),
                side: r.side.clone(),
            })
    }
}

impl From<&OrderItem> for OrderRow {
    fn from(item: &OrderItem) -> Self {
        Self {
            client_order_id: item.client_order_id.clone(),
            time: extract_time(&item.created_at).to_string(),
            full_timestamp: item.created_at.clone(),
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

/// One OHLCV bar for the chart page. `time` stays the raw RFC3339 string
/// from the wire/REST response — same "never reparse the wire format" rule
/// as `FeedRow::time` — the JS chart layer parses it, not Rust. Derives
/// `Serialize` so the chart page can hand a bar straight to `document::eval`
/// as JSON without a separate wire-shape struct.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Candle {
    pub time: String,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

impl From<&CandleMessage> for Candle {
    fn from(msg: &CandleMessage) -> Self {
        Self {
            time: msg.time.clone(),
            open: msg.open,
            high: msg.high,
            low: msg.low,
            close: msg.close,
            volume: msg.volume,
        }
    }
}

impl From<&CandleItem> for Candle {
    fn from(item: &CandleItem) -> Self {
        Self {
            time: item.time.clone(),
            open: item.open,
            high: item.high,
            low: item.low,
            close: item.close,
            volume: item.volume,
        }
    }
}

/// Cap on candles retained per `(exchange, pair, interval)` series —
/// mirrors the engine's `CANDLE_HISTORY_CAPACITY`, so a long-running chart
/// page doesn't grow the series vector unboundedly.
pub const MAX_CANDLE_ROWS: usize = 500;

/// Per-`(exchange, pair, interval)` OHLCV series for the chart page. Two
/// write paths: `seed` (full replace from `GET /v1/candles` on page load or
/// pair/interval switch) and `apply` (incremental, from the live WS feed —
/// updates the last bar in place if its `time` is unchanged, since the
/// engine can rebroadcast the same in-progress bucket before it closes;
/// otherwise appends a new bar on the next interval boundary).
#[derive(Debug, Clone, Default)]
pub struct CandleStore {
    series: HashMap<String, Vec<Candle>>,
}

impl CandleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&mut self, key: &str, candles: Vec<Candle>) {
        self.series.insert(key.to_string(), candles);
    }

    pub fn apply(&mut self, msg: &CandleMessage) {
        let key = msg.key();
        let candle = Candle::from(msg);
        let series = self.series.entry(key).or_default();
        match series.last_mut() {
            Some(last) if last.time == candle.time => *last = candle,
            // A candle older than the series' last bar is a stale/reordered
            // WS message (network reorder, or a forming-bar broadcast for a
            // selection the user has since switched away from) — dropped
            // rather than appended, which would otherwise corrupt the
            // series' chronological order.
            Some(last) if candle.time < last.time => {}
            _ => {
                series.push(candle);
                if series.len() > MAX_CANDLE_ROWS {
                    series.remove(0);
                }
            }
        }
    }

    pub fn series_for(&self, key: &str) -> &[Candle] {
        self.series.get(key).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Holds the most recent backtest run so the chart page can read it
/// regardless of which page triggered `POST /v1/backtest/run` — there is no
/// dedicated Backtest page yet, so this is the first piece of that future
/// store, scoped to what the chart's trade overlay needs today.
#[derive(Debug, Clone, Default)]
pub struct BacktestStore {
    pub result: Option<BacktestResult>,
}

impl BacktestStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, result: BacktestResult) {
        self.result = Some(result);
    }

    pub fn clear(&mut self) {
        self.result = None;
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

    #[test]
    fn order_store_open_position_renders_entry_marker_only() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "filled"));

        let marker = store
            .open_position_for("tabdeal", "USDT/IRT")
            .expect("a filled order for this exchange/pair must produce a marker");

        assert_eq!(marker.side, "buy");
        assert_eq!(marker.time, "2026-06-21T10:00:00Z");
        // Intentionally partial — stop_loss/take_profit don't exist on
        // OrderUpdatePayload yet, so there is no rectangle/SL/TP line to
        // render, only this single entry marker.
    }

    #[test]
    fn order_store_open_position_ignores_cancelled_orders() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "cancelled"));

        assert!(store.open_position_for("tabdeal", "USDT/IRT").is_none());
    }

    #[test]
    fn order_store_open_position_ignores_other_pairs() {
        let mut store = OrderStore::new();
        store.apply(&order("cli-1", "filled"));

        assert!(store.open_position_for("tabdeal", "BTC/IRT").is_none());
    }

    // --- CandleStore tests ---

    fn candle_msg(time: &str, close: u64) -> CandleMessage {
        CandleMessage {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: time.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
        }
    }

    #[test]
    fn candle_store_seed_replaces_existing_history() {
        let mut store = CandleStore::new();
        store.seed(
            "tabdeal:USDT/IRT:1m",
            vec![Candle {
                time: "t0".to_string(),
                open: 1,
                high: 1,
                low: 1,
                close: 1,
                volume: 1,
            }],
        );
        store.seed(
            "tabdeal:USDT/IRT:1m",
            vec![Candle {
                time: "t1".to_string(),
                open: 2,
                high: 2,
                low: 2,
                close: 2,
                volume: 1,
            }],
        );

        let series = store.series_for("tabdeal:USDT/IRT:1m");
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].time, "t1");
    }

    #[test]
    fn candle_store_apply_pushes_new_bucket() {
        let mut store = CandleStore::new();
        store.apply(&candle_msg("t0", 100));
        store.apply(&candle_msg("t1", 200));

        let series = store.series_for("tabdeal:USDT/IRT:1m");
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].close, 100);
        assert_eq!(series[1].close, 200);
    }

    #[test]
    fn candle_store_apply_updates_in_place_when_time_unchanged() {
        let mut store = CandleStore::new();
        store.apply(&candle_msg("t0", 100));
        store.apply(&candle_msg("t0", 150));

        let series = store.series_for("tabdeal:USDT/IRT:1m");
        assert_eq!(series.len(), 1, "same time bucket must update in place");
        assert_eq!(series[0].close, 150);
    }

    #[test]
    fn candle_store_apply_caps_length() {
        let mut store = CandleStore::new();
        for i in 0..(MAX_CANDLE_ROWS + 5) {
            // Zero-padded so the strings sort the same as the numbers they
            // represent — this codebase's RFC3339 timestamps are always
            // fixed-width for exactly this reason (see `visible_trades`'s
            // doc comment in `pages/chart.rs`); a non-padded `"t{i}"` would
            // make `"t10"` sort before `"t9"` and trip the new out-of-order
            // rejection below.
            store.apply(&candle_msg(&format!("t{i:04}"), i as u64));
        }

        let series = store.series_for("tabdeal:USDT/IRT:1m");
        assert_eq!(series.len(), MAX_CANDLE_ROWS);
        assert_eq!(series[0].close, 5, "oldest entries must be evicted");
    }

    #[test]
    fn candle_store_apply_ignores_a_candle_older_than_the_last_bar() {
        let mut store = CandleStore::new();
        store.apply(&candle_msg("t1", 100));
        store.apply(&candle_msg("t2", 200));
        // A stale/reordered WS message for an already-passed bucket must
        // not be appended after the newer bar — that would corrupt the
        // series' chronological order (and break `lightweight-charts`,
        // which requires strictly increasing `time` on `series.update()`).
        store.apply(&candle_msg("t0", 999));

        let series = store.series_for("tabdeal:USDT/IRT:1m");
        assert_eq!(series.len(), 2, "the stale t0 candle must be dropped");
        assert_eq!(series[0].time, "t1");
        assert_eq!(series[1].time, "t2");
    }

    #[test]
    fn candle_store_series_for_unknown_key_returns_empty() {
        let store = CandleStore::new();
        assert!(store.series_for("does:not:exist").is_empty());
    }

    fn backtest_result(win_rate: f64) -> BacktestResult {
        BacktestResult {
            strategy_id: "s1".to_string(),
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            candle_count: 10,
            signal_count: 1,
            total_return_pct: 1.0,
            max_drawdown_pct: 0.1,
            trade_log: vec![],
            signal_log: vec![],
            closed_trades: vec![],
            win_rate,
            avg_rr: None,
        }
    }

    #[test]
    fn backtest_store_set_replaces_previous_result() {
        let mut store = BacktestStore::new();
        store.set(backtest_result(0.5));
        store.set(backtest_result(0.75));

        assert_eq!(store.result.unwrap().win_rate, 0.75);
    }

    #[test]
    fn backtest_store_clear_empties_result() {
        let mut store = BacktestStore::new();
        store.set(backtest_result(0.5));

        store.clear();

        assert!(store.result.is_none());
    }
}
