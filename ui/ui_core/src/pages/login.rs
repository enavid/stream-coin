use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::auth::Session;
use crate::router::Route;
use crate::state::AppState;

#[component]
pub fn Login(server_url: String) -> Element {
    let mut state = use_context::<AppState>();
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut submitting = use_signal(|| false);

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = ApiClient::new(server_url.clone());
        let user = username();
        let pass = password();
        submitting.set(true);
        spawn(async move {
            let result = api.login(&user, &pass).await;
            submitting.set(false);
            match result {
                Ok(token_resp) => match Session::from_token(token_resp.token) {
                    Ok(session) => {
                        state.set_session(session);
                        state.navigate(Route::Dashboard);
                    }
                    Err(_) => error.set(Some("Server returned an invalid token".to_string())),
                },
                Err(msg) => error.set(Some(msg)),
            }
        });
    };

    rsx! {
        div { id: "login-screen",
            div { class: "login-card",
                div { class: "login-logo",
                    div { class: "logo-mark", "⚡" }
                    div { class: "title", "stream-coin" }
                    div { class: "sub", "sign in to continue" }
                }
                form { onsubmit: on_submit,
                    div { class: "field",
                        label { "Username" }
                        input {
                            class: "finput",
                            value: "{username}",
                            oninput: move |evt| username.set(evt.value()),
                        }
                    }
                    div { class: "field",
                        label { "Password" }
                        input {
                            class: "finput",
                            r#type: "password",
                            value: "{password}",
                            oninput: move |evt| password.set(evt.value()),
                        }
                    }
                    if let Some(err) = error() {
                        div { class: "form-error", "{err}" }
                    }
                    button {
                        class: "btn btn-primary",
                        r#type: "submit",
                        disabled: submitting(),
                        if submitting() { "Signing in…" } else { "Sign in" }
                    }
                }
            }
        }
    }
}
