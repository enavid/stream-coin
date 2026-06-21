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
}
