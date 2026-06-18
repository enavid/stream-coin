use dioxus::prelude::*;

use ui_core::api::ApiClient;
use ui_core::state::provide_app_state;
use ui_core::Dashboard;

mod ws;

const MAIN_CSS: Asset = asset!("/assets/main.css");

/// Backend base URL. Hardcoded for now — promoting this to a runtime
/// setting (e.g. a settings page backed by local storage) is a small,
/// isolated follow-up since [`Dashboard`] already takes it as a prop.
const SERVER_URL: &str = "http://localhost:8080";

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let state = provide_app_state();

    use_future(move || async move {
        let ws_url = ApiClient::new(SERVER_URL).ws_url();
        ws::connect_and_listen(ws_url, state).await;
    });

    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        Dashboard { server_url: SERVER_URL.to_string() }
    }
}
