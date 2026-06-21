mod admin;
mod backtest;
mod chart;
mod login;
mod orders;
mod settings;
mod strategies;

pub use admin::Admin;
pub use backtest::Backtest;
pub use chart::Chart;
pub use login::Login;
pub use orders::Orders;
pub use settings::Settings;
pub use strategies::Strategies;

use dioxus::prelude::*;

use crate::state::AppState;

/// Every authenticated page needs the current JWT to call `ApiClient`.
/// Reads fresh each call (rather than being captured once) so it always
/// reflects the latest session, e.g. right after login.
pub(crate) fn current_token(state: &AppState) -> Option<String> {
    state.session.read().as_ref().map(|s| s.token.clone())
}
