use dioxus::prelude::*;

#[component]
pub fn Header(connected: bool, server_url: String) -> Element {
    rsx! {
        header {
            div { class: "logo",
                div { class: "logo-mark", "⚡" }
                "stream-coin"
            }
            div { class: "header-end",
                span { class: "server-chip", "{server_url}" }
                div { class: "ws-pill",
                    div { class: if connected { "ws-dot" } else { "ws-dot disconnected" } }
                    span {
                        if connected {
                            "Connected"
                        } else {
                            "Disconnected"
                        }
                    }
                }
            }
        }
    }
}
