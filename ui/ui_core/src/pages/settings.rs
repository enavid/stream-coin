use dioxus::prelude::*;

use super::current_token;
use crate::api::{ApiClient, CredentialSummaryResponse};
use crate::state::AppState;

#[component]
pub fn Settings(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| {
        ApiClient::new(server_url).with_unauthorized_handler(move || {
            let mut state = state;
            state.clear_session();
        })
    });

    let can_write = state
        .session
        .read()
        .as_ref()
        .map(|s| s.has("exchange_credentials.write"))
        .unwrap_or(false);

    let mut credentials = use_signal(Vec::<CredentialSummaryResponse>::new);
    let mut load_error = use_signal(|| None::<String>);

    let refresh = move || {
        let api = api();
        let token = current_token(&state);
        spawn(async move {
            let Some(token) = token else { return };
            match api.list_own_credentials(&token).await {
                Ok(resp) => credentials.set(resp.credentials),
                Err(e) => load_error.set(Some(e)),
            }
        });
    };

    use_future(move || {
        refresh();
        async move {}
    });

    let mut exchange_choice = use_signal(String::new);
    let mut api_key = use_signal(String::new);
    let mut secret = use_signal(String::new);
    let mut save_error = use_signal(|| None::<String>);

    let catalog = state.catalog.read();
    let exchanges = catalog.exchanges().to_vec();
    let selected_exchange = catalog.resolve_exchange(&exchange_choice());
    drop(catalog);

    let exchange_for_save = selected_exchange.clone();
    let on_save = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let body = serde_json::json!({ "api_key": api_key(), "secret": secret() });
        let exchange = exchange_for_save.clone();
        spawn(async move {
            match api.set_own_credentials(&token, &exchange, body).await {
                Ok(()) => {
                    save_error.set(None);
                    api_key.set(String::new());
                    secret.set(String::new());
                    refresh();
                }
                Err(e) => save_error.set(Some(e)),
            }
        });
    };

    let on_delete = move |exchange: String| {
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        spawn(async move {
            if api.delete_own_credentials(&token, &exchange).await.is_ok() {
                refresh();
            }
        });
    };

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Settings" }
                div { class: "page-sub", "Your own exchange API keys — encrypted at rest, never shown again" }
            }
        }

        if let Some(err) = load_error() {
            div { class: "form-error", "{err}" }
        }

        section { class: "block",
            span { class: "label", "Configured Exchanges" }
            div { class: "field-row grid-2",
                for c in credentials() {
                    div { class: "card", key: "{c.exchange}", style: "display:flex; align-items:center; justify-content:space-between;",
                        div {
                            div { style: "font-weight:700; color:#fff;", "{c.exchange}" }
                            div { class: "mono", style: "font-size:11px; color:var(--muted2); margin-top:4px;", "configured · {c.created_at}" }
                        }
                        if can_write {
                            button {
                                class: "btn btn-danger btn-sm",
                                onclick: {
                                    let ex = c.exchange.clone();
                                    move |_| on_delete(ex.clone())
                                },
                                "Remove"
                            }
                        }
                    }
                }
            }
        }

        if can_write {
            section { class: "block card",
                span { class: "label", style: "margin-bottom:14px;", "Add / Update Credentials" }
                form { onsubmit: on_save,
                    div { class: "field-row grid-2", style: "margin-bottom:10px;",
                        div { class: "field",
                            label { "Exchange" }
                            select {
                                class: "finput",
                                value: "{selected_exchange}",
                                onchange: move |e| exchange_choice.set(e.value()),
                                for ex in exchanges.iter() { option { value: "{ex.name}", "{ex.name}" } }
                            }
                        }
                        div { class: "field",
                            label { "API Key" }
                            input { class: "finput", value: "{api_key}", oninput: move |e| api_key.set(e.value()) }
                        }
                    }
                    div { class: "field", style: "margin-bottom:14px;",
                        label { "Secret" }
                        input { class: "finput", r#type: "password", value: "{secret}", oninput: move |e| secret.set(e.value()) }
                    }
                    if let Some(err) = save_error() {
                        div { class: "form-error", "{err}" }
                    }
                    button { class: "btn btn-primary", r#type: "submit", "Save credentials" }
                }
            }
        }
    }
}
