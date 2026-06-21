mod store;

pub use store::{
    FeedRow, OrderRow, OrderStore, SignalRow, SignalStore, TickerStore, MAX_SIGNAL_ROWS,
};

use dioxus::prelude::*;

use crate::auth::Session;
use crate::protocol::{OrderUpdateMessage, PriceMessage, SignalMessage};
use crate::router::Route;

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
        }
    }

    pub fn apply_price(&mut self, msg: &PriceMessage) {
        self.store.write().apply(msg);
    }

    pub fn remove_ticker(&mut self, key: &str) {
        self.store.write().remove(key);
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

    pub fn navigate(&mut self, route: Route) {
        self.route.set(route);
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
