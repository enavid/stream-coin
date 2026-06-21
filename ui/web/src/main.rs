use dioxus::prelude::*;

use ui_core::api::ApiClient;
use ui_core::state::provide_app_state;
use ui_core::AppShell;

mod browser;
mod ws;

const MAIN_CSS: Asset = asset!("/assets/main.css");

/// Backend base URL. Hardcoded for now — promoting this to a runtime
/// setting (e.g. a settings page backed by local storage) is a small,
/// isolated follow-up since every page already takes it as a prop.
const SERVER_URL: &str = "http://localhost:8080";

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut state = provide_app_state();

    // Runs once on mount — restores whatever a previous page load left in
    // the URL/localStorage, and starts the popstate listener that keeps
    // `state.route` in sync with browser back/forward for the rest of the
    // page's lifetime.
    browser::restore_session(&mut state);
    browser::restore_route(&mut state);
    browser::restore_theme(&mut state);
    browser::listen_popstate(state);

    use_effect(move || {
        let token = (state.session)().map(|s| s.token);
        browser::persist_session(token.as_deref());
    });

    use_effect(move || {
        browser::persist_theme((state.theme)());
    });

    use_effect(move || {
        browser::sync_url((state.route)());
    });

    use_future(move || async move {
        ws::connect_and_listen(ApiClient::new(SERVER_URL), state).await;
    });

    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        AppShell { server_url: SERVER_URL.to_string() }
    }
}
