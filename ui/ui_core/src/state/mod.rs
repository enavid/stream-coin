mod catalog;
mod store;
pub mod playback;

pub use catalog::ExchangeCatalog;
pub use playback::{PlaybackSpeed, PlaybackState};
pub use store::{
    BacktestStore, Candle, CandleStore, FeedRow, OrderRow, OrderStore, SignalRow, SignalStore,
    TickerStore, MAX_SIGNAL_ROWS,
};

use dioxus::prelude::*;

use crate::api::{BacktestResult, CandleItem, ExchangeResponse, PairResponse};
use crate::auth::Session;
use crate::protocol::{CandleMessage, OrderUpdateMessage, PriceMessage, SignalMessage};
use crate::router::Route;
use crate::theme::Theme;

/// Reactive app state, provided via Dioxus context so any component in
/// the tree can read tickers/feed/signals/orders, the current route, or
/// the session without prop drilling. Business logic lives in the
/// signal-free stores (`TickerStore`, `SignalStore`, `OrderStore`) and in
/// [`Session`] — all unit tested; this wrapper only adds `Signal` plumbing,
/// same as the existing `store`/`connected` fields.
#[derive(Clone, Copy)]
pub struct AppState {
    pub store: Signal<TickerStore>,
    pub connected: Signal<bool>,
    pub route: Signal<Route>,
    pub session: Signal<Option<Session>>,
    pub signals: Signal<SignalStore>,
    pub orders: Signal<OrderStore>,
    pub catalog: Signal<ExchangeCatalog>,
    pub theme: Signal<Theme>,
    pub candles: Signal<CandleStore>,
    pub backtest: Signal<BacktestStore>,
    /// Chart playback cursor state (Loop 6i) — shared so the chart toolbar
    /// and the timer effect both read and write the same signal.
    pub playback: Signal<PlaybackState>,
    /// Bumped by the platform's WS transport every time it reconnects
    /// *after* a previous disconnect (not on the very first connect) —
    /// per-page fetch effects that also read this re-run their REST fetch,
    /// so a connection drop doesn't leave local state silently stale
    /// forever. See `ROADMAP.md`'s API standard: "after reconnection, treat
    /// local state as stale and reconcile via REST."
    pub resync_epoch: Signal<u32>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            store: Signal::new(TickerStore::new()),
            connected: Signal::new(false),
            route: Signal::new(Route::Dashboard),
            session: Signal::new(None),
            signals: Signal::new(SignalStore::new()),
            orders: Signal::new(OrderStore::new()),
            catalog: Signal::new(ExchangeCatalog::new()),
            theme: Signal::new(Theme::default()),
            candles: Signal::new(CandleStore::new()),
            backtest: Signal::new(BacktestStore::new()),
            playback: Signal::new(PlaybackState::new()),
            resync_epoch: Signal::new(0),
        }
    }

    /// Call after a WS reconnect that follows a previous disconnect (not
    /// the very first connect — pages already do a normal fetch on mount).
    pub fn mark_resynced(&mut self) {
        let next = (self.resync_epoch)() + 1;
        self.resync_epoch.set(next);
    }

    pub fn apply_price(&mut self, msg: &PriceMessage) {
        self.store.write().apply(msg);
    }

    pub fn remove_ticker(&mut self, key: &str) {
        self.store.write().remove(key);
    }

    /// See [`TickerStore::clear_flash`] — called by the platform's WS
    /// transport a short delay after each price tick so the up/down
    /// highlight fades instead of sticking permanently.
    pub fn clear_flash(&mut self, key: &str) {
        self.store.write().clear_flash(key);
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected.set(connected);
    }

    pub fn apply_signal(&mut self, msg: &SignalMessage) {
        self.signals.write().apply(msg);
    }

    pub fn apply_order_update(&mut self, msg: &OrderUpdateMessage) {
        self.orders.write().apply(msg);
    }

    pub fn apply_candle(&mut self, msg: &CandleMessage) {
        self.candles.write().apply(msg);
    }

    /// Seeds a `(exchange, pair, interval)` series from `GET /v1/candles` —
    /// the chart page calls this on mount and on every selector change.
    pub fn seed_candles(&mut self, key: &str, items: &[CandleItem]) {
        self.candles
            .write()
            .seed(key, items.iter().map(Candle::from).collect());
    }

    pub fn set_backtest_result(&mut self, result: BacktestResult) {
        self.backtest.write().set(result);
    }

    pub fn clear_backtest_result(&mut self) {
        self.backtest.write().clear();
    }

    pub fn set_exchanges(&mut self, exchanges: Vec<ExchangeResponse>) {
        self.catalog.write().set_exchanges(exchanges);
    }

    pub fn set_pairs_for(&mut self, exchange: &str, pairs: Vec<PairResponse>) {
        self.catalog.write().set_pairs(exchange, pairs);
    }

    pub fn navigate(&mut self, route: Route) {
        self.route.set(route);
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme.set(theme);
    }

    pub fn toggle_theme(&mut self) {
        let next = (self.theme)().toggled();
        self.theme.set(next);
    }

    pub fn set_session(&mut self, session: Session) {
        self.session.set(Some(session));
    }

    /// Also navigates to [`Route::Login`] — every caller that clears the
    /// session (logout, a 401 from an expired token) wants to land back
    /// on the login screen, not be left on a now-unauthorized page.
    pub fn clear_session(&mut self) {
        self.session.set(None);
        self.navigate(Route::Login);
    }
}

/// Installs [`AppState`] into the current component's context. Call once
/// near the platform's root component; [`crate::Dashboard`] and any
/// descendant reads it with `use_context::<AppState>()`.
pub fn provide_app_state() -> AppState {
    use_context_provider(AppState::new)
}
