use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::icons::{
    IconAdmin, IconBacktest, IconChart, IconDashboard, IconLogout, IconMenu, IconMoon, IconOrders,
    IconSettings, IconStrategy, IconSun,
};
use crate::pages::{Admin, Backtest, Chart, Login, Orders, Settings, Strategies};
use crate::router::Route;
use crate::state::AppState;
use crate::theme::Theme;
use crate::Dashboard;

struct NavItem {
    route: Route,
    label: &'static str,
    icon: fn() -> Element,
    /// `None` means visible to every authenticated user.
    requires: Option<&'static str>,
}

/// The day-to-day destinations a trader actually works from.
const PRIMARY_NAV_ITEMS: &[NavItem] = &[
    NavItem {
        route: Route::Dashboard,
        label: "Dashboard",
        icon: || rsx! { IconDashboard {} },
        requires: None,
    },
    NavItem {
        route: Route::Chart,
        label: "Chart",
        icon: || rsx! { IconChart {} },
        requires: None,
    },
    NavItem {
        route: Route::Strategies,
        label: "Strategies",
        icon: || rsx! { IconStrategy {} },
        requires: None,
    },
    NavItem {
        route: Route::Backtest,
        label: "Backtest",
        icon: || rsx! { IconBacktest {} },
        requires: None,
    },
    NavItem {
        route: Route::Orders,
        label: "Orders",
        icon: || rsx! { IconOrders {} },
        requires: None,
    },
];

/// Account/admin utilities — not a daily destination, so every common
/// sidebar pattern (Slack, Notion, Linear, Discord) pins these to the
/// bottom, separated from the primary nav, instead of mixing them in.
const SECONDARY_NAV_ITEMS: &[NavItem] = &[
    NavItem {
        route: Route::Admin,
        label: "Users & Roles",
        icon: || rsx! { IconAdmin {} },
        requires: Some("users.manage"),
    },
    NavItem {
        route: Route::Settings,
        label: "Settings",
        icon: || rsx! { IconSettings {} },
        requires: Some("exchange_credentials.write"),
    },
];

/// Topbar + sidebar + routed content. Renders [`Login`] full-screen
/// instead when there is no session — the route guard for this app.
#[component]
pub fn AppShell(server_url: String) -> Element {
    let mut state = use_context::<AppState>();
    let mut mobile_nav_open = use_signal(|| false);
    let api = use_signal(|| ApiClient::new(server_url.clone()));

    // Registered unconditionally (must run on every render, login or not —
    // Dioxus requires a stable hook call sequence per component instance)
    // but only does anything once a session exists. Reads `state.session`
    // reactively, so it re-fetches whenever the session changes: after
    // login, after a different user logs in, etc. The exchange list and
    // every exchange's active pairs are loaded once here rather than by
    // each page separately, so "Strategies"/"Backtest"/"Orders"/the
    // dashboard's "Add Ticker" form all suggest the same live set.
    use_effect(move || {
        let session = (state.session)();
        let api = api();
        spawn(async move {
            let Some(session) = session else { return };
            let Ok(resp) = api.list_exchanges(&session.token).await else {
                return;
            };
            state.set_exchanges(resp.exchanges.clone());
            for exchange in resp.exchanges {
                if let Ok(pairs) = api
                    .list_exchange_pairs(&session.token, &exchange.name)
                    .await
                {
                    state.set_pairs_for(&exchange.name, pairs.pairs);
                }
            }
        });
    });

    let Some(session) = (state.session)() else {
        return rsx! { Login { server_url: server_url.clone() } };
    };

    let connected = (state.connected)();
    let current_route = (state.route)();
    let theme = (state.theme)();

    let on_logout = move |_| state.clear_session();
    let on_toggle_theme = move |_| state.toggle_theme();

    let render_nav_item = move |item: &'static NavItem| {
        rsx! {
            a {
                key: "{item.label}",
                class: if current_route == item.route { "nav-link active" } else { "nav-link" },
                onclick: {
                    let route = item.route;
                    move |_| {
                        state.navigate(route);
                        mobile_nav_open.set(false);
                    }
                },
                span { class: "ic", { (item.icon)() } }
                "{item.label}"
            }
        }
    };

    rsx! {
        div { id: "app", class: "active", "data-theme": theme.as_str(),
            header { class: "topbar",
                div { class: "logo",
                    button {
                        class: "btn-ghost btn-sm hamburger-btn",
                        onclick: move |_| mobile_nav_open.set(!mobile_nav_open()),
                        IconMenu {}
                    }
                    div { class: "logo-mark", "⚡" }
                    "stream-coin"
                }
                div { class: "header-end",
                    div { class: "ws-pill",
                        div { class: if connected { "ws-dot" } else { "ws-dot disconnected" } }
                        span { if connected { "Connected" } else { "Disconnected" } }
                    }
                    button {
                        class: "theme-toggle",
                        title: if theme == Theme::Dark { "Switch to light theme" } else { "Switch to dark theme" },
                        onclick: on_toggle_theme,
                        if theme == Theme::Dark { IconSun {} } else { IconMoon {} }
                    }
                    div { class: "user-chip",
                        div { class: "avatar", "{session.user_id.chars().next().unwrap_or('U')}" }
                        div { class: "user-meta",
                            span { class: "user-name", "User {session.user_id}" }
                            span { class: "user-role", "{session.role_label()}" }
                        }
                    }
                    button { class: "logout-btn", onclick: on_logout, IconLogout {} }
                }
            }

            nav { class: if mobile_nav_open() { "sidebar open" } else { "sidebar" },
                for item in PRIMARY_NAV_ITEMS.iter() {
                    if item.requires.is_none_or(|p| session.has(p)) {
                        {render_nav_item(item)}
                    }
                }
                div { class: "sidebar-bottom",
                    if SECONDARY_NAV_ITEMS.iter().any(|item| item.requires.is_none_or(|p| session.has(p))) {
                        div { class: "nav-sep" }
                        for item in SECONDARY_NAV_ITEMS.iter() {
                            if item.requires.is_none_or(|p| session.has(p)) {
                                {render_nav_item(item)}
                            }
                        }
                    }
                    div { class: "sidebar-footer",
                        div { class: "sidebar-status",
                            div { class: if connected { "ws-dot" } else { "ws-dot disconnected" } }
                            span { if connected { "Engine online" } else { "Engine offline" } }
                        }
                        span { class: "sidebar-version", "stream-coin · v0.1" }
                    }
                }
            }

            main { class: "content",
                match current_route {
                    Route::Login => rsx! { Login { server_url: server_url.clone() } },
                    Route::Dashboard => rsx! { Dashboard { server_url: server_url.clone() } },
                    Route::Chart => rsx! { Chart { server_url: server_url.clone() } },
                    Route::Strategies => rsx! { Strategies { server_url: server_url.clone() } },
                    Route::Backtest => rsx! { Backtest { server_url: server_url.clone() } },
                    Route::Orders => rsx! { Orders { server_url: server_url.clone() } },
                    Route::Admin => rsx! { Admin { server_url: server_url.clone() } },
                    Route::Settings => rsx! { Settings { server_url: server_url.clone() } },
                }
            }
        }
    }
}
