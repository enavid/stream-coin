//! Browser-only glue: `localStorage` session persistence and syncing
//! [`Route`] with the real URL (history API, `popstate`). `ui_core` stays
//! platform-agnostic — same reasoning as `ws.rs` owning the WebSocket
//! transport — so this is the one place in the workspace allowed to touch
//! `web_sys::window()`.

use wasm_bindgen::prelude::*;

use ui_core::auth::Session;
use ui_core::router::Route;
use ui_core::state::AppState;

const SESSION_STORAGE_KEY: &str = "stream_coin_session_token";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Restores a previously persisted session on boot, if the stored JWT is
/// still well-formed (it may have expired — the server is the one that
/// actually enforces that on the next request).
pub fn restore_session(state: &mut AppState) {
    let Some(storage) = local_storage() else {
        return;
    };
    let Ok(Some(token)) = storage.get_item(SESSION_STORAGE_KEY) else {
        return;
    };
    if let Ok(session) = Session::from_token(token) {
        state.set_session(session);
    }
}

/// Call once after every session change (login/logout) to keep
/// `localStorage` in sync so a page refresh doesn't lose the session.
pub fn persist_session(token: Option<&str>) {
    let Some(storage) = local_storage() else {
        return;
    };
    match token {
        Some(token) => {
            let _ = storage.set_item(SESSION_STORAGE_KEY, token);
        }
        None => {
            let _ = storage.remove_item(SESSION_STORAGE_KEY);
        }
    }
}

fn current_path() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_else(|| "/".to_string())
}

/// Sets the initial route from the real URL on boot — a refresh on
/// `/strategies` must land back on `/strategies`, not bounce to `/`.
pub fn restore_route(state: &mut AppState) {
    state.navigate(Route::from_path(&current_path()));
}

/// Pushes the current in-app route into the URL bar, skipping the push if
/// the URL already matches (avoids feedback loops with `listen_popstate`
/// and duplicate back-stack entries when nothing actually changed).
pub fn sync_url(route: Route) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if current_path() == route.path() {
        return;
    }
    let Ok(history) = window.history() else {
        return;
    };
    let _ = history.push_state_with_url(&JsValue::NULL, "", Some(route.path()));
}

/// Listens for browser back/forward and updates `state.route` to match.
/// The closure is leaked intentionally — it must live for the lifetime of
/// the page, which for a single-page wasm app is "forever".
pub fn listen_popstate(mut state: AppState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::<dyn FnMut()>::new(move || {
        state.navigate(Route::from_path(&current_path()));
    });
    let _ = window.add_event_listener_with_callback("popstate", closure.as_ref().unchecked_ref());
    closure.forget();
}
