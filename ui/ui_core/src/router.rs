//! Minimal hand-rolled router. `dioxus` is pinned to `0.7` workspace-wide and
//! the only published `dioxus-router` is a `0.8.0-alpha.0` tied to a
//! different dioxus line — not worth pulling into a trading app for 7 flat
//! routes with no dynamic segments. `ui/web` owns syncing this with the
//! real browser URL (history API, popstate) since `ui_core` stays
//! platform-agnostic.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Login,
    Dashboard,
    Chart,
    Strategies,
    Backtest,
    Orders,
    Admin,
    Settings,
}

impl Route {
    pub fn path(&self) -> &'static str {
        match self {
            Route::Login => "/login",
            Route::Dashboard => "/",
            Route::Chart => "/chart",
            Route::Strategies => "/strategies",
            Route::Backtest => "/backtest",
            Route::Orders => "/orders",
            Route::Admin => "/admin",
            Route::Settings => "/settings",
        }
    }

    /// Unknown paths default to [`Route::Dashboard`] rather than erroring —
    /// there is no 404 page in this app.
    pub fn from_path(path: &str) -> Self {
        match path {
            "/login" => Route::Login,
            "/chart" => Route::Chart,
            "/strategies" => Route::Strategies,
            "/backtest" => Route::Backtest,
            "/orders" => Route::Orders,
            "/admin" => Route::Admin,
            "/settings" => Route::Settings,
            _ => Route::Dashboard,
        }
    }

    /// The permission required to view this route's *content*, if any.
    /// `app_shell.rs` is the single source of truth that reads this for
    /// both the nav link visibility *and* the actual content guard —
    /// hiding a nav link doesn't stop a user from typing the path directly
    /// into the URL bar, so the content match must check this too, not
    /// just the nav rendering.
    pub fn required_permission(&self) -> Option<&'static str> {
        match self {
            Route::Admin => Some("users.manage"),
            Route::Settings => Some("exchange_credentials.write"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_path_round_trips_through_from_path() {
        let routes = [
            Route::Login,
            Route::Dashboard,
            Route::Chart,
            Route::Strategies,
            Route::Backtest,
            Route::Orders,
            Route::Admin,
            Route::Settings,
        ];
        for route in routes {
            assert_eq!(Route::from_path(route.path()), route);
        }
    }

    #[test]
    fn from_path_defaults_to_dashboard_for_unknown_path() {
        assert_eq!(Route::from_path("/does-not-exist"), Route::Dashboard);
    }

    #[test]
    fn dashboard_path_is_root() {
        assert_eq!(Route::Dashboard.path(), "/");
    }

    #[test]
    fn required_permission_is_some_for_admin_and_settings() {
        assert_eq!(Route::Admin.required_permission(), Some("users.manage"));
        assert_eq!(
            Route::Settings.required_permission(),
            Some("exchange_credentials.write")
        );
    }

    #[test]
    fn required_permission_is_none_for_routes_open_to_every_authenticated_user() {
        for route in [
            Route::Login,
            Route::Dashboard,
            Route::Chart,
            Route::Strategies,
            Route::Backtest,
            Route::Orders,
        ] {
            assert_eq!(route.required_permission(), None);
        }
    }
}
