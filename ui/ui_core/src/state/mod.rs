mod store;

pub use store::{FeedRow, TickerStore, MAX_FEED_ROWS};

use dioxus::prelude::*;

use crate::protocol::PriceMessage;

/// Reactive app state, provided via Dioxus context so any component in
/// the tree can read tickers/feed or react to connection status without
/// prop drilling. Business logic lives in [`TickerStore`] (signal-free,
/// unit tested); this wrapper only adds the `Signal` plumbing.
#[derive(Clone, Copy)]
pub struct AppState {
    pub store: Signal<TickerStore>,
    pub connected: Signal<bool>,
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
}

/// Installs [`AppState`] into the current component's context. Call once
/// near the platform's root component; [`crate::Dashboard`] and any
/// descendant reads it with `use_context::<AppState>()`.
pub fn provide_app_state() -> AppState {
    use_context_provider(AppState::new)
}
