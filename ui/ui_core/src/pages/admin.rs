use dioxus::prelude::*;

use super::current_token;
use crate::api::{ApiClient, RoleResponse, UserResponse};
use crate::state::AppState;

#[component]
pub fn Admin(server_url: String) -> Element {
    let state = use_context::<AppState>();
    let api = use_signal(|| ApiClient::new(server_url));

    let has_users_manage = state
        .session
        .read()
        .as_ref()
        .map(|s| s.has("users.manage"))
        .unwrap_or(false);
    let has_roles_manage = state
        .session
        .read()
        .as_ref()
        .map(|s| s.has("roles.manage"))
        .unwrap_or(false);

    let mut users = use_signal(Vec::<UserResponse>::new);
    let mut roles = use_signal(Vec::<RoleResponse>::new);
    let mut permissions = use_signal(Vec::<String>::new);
    let mut load_error = use_signal(|| None::<String>);

    let refresh = move || {
        let api = api();
        let token = current_token(&state);
        spawn(async move {
            let Some(token) = token else { return };
            match api.list_users(&token).await {
                Ok(resp) => users.set(resp.users),
                Err(e) => load_error.set(Some(e)),
            }
            if let Ok(resp) = api.list_roles(&token).await {
                roles.set(resp.roles);
            }
            if let Ok(resp) = api.list_permissions(&token).await {
                permissions.set(resp.permissions);
            }
        });
    };

    use_future(move || {
        refresh();
        async move {}
    });

    let mut new_username = use_signal(String::new);
    let mut new_password = use_signal(String::new);
    let mut new_roles = use_signal(String::new);
    let mut create_user_error = use_signal(|| None::<String>);

    let on_create_user = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let role_list: Vec<String> = new_roles()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        spawn(async move {
            match api
                .create_user(&token, &new_username(), &new_password(), role_list)
                .await
            {
                Ok(_) => {
                    create_user_error.set(None);
                    new_username.set(String::new());
                    new_password.set(String::new());
                    new_roles.set(String::new());
                    refresh();
                }
                Err(e) => create_user_error.set(Some(e)),
            }
        });
    };

    let mut new_role_name = use_signal(String::new);
    let mut new_role_permissions = use_signal(String::new);
    let mut create_role_error = use_signal(|| None::<String>);

    let on_create_role = move |evt: Event<FormData>| {
        evt.prevent_default();
        let api = api();
        let Some(token) = current_token(&state) else {
            return;
        };
        let perms: Vec<String> = new_role_permissions()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        spawn(async move {
            match api.create_role(&token, &new_role_name(), perms).await {
                Ok(()) => {
                    create_role_error.set(None);
                    new_role_name.set(String::new());
                    new_role_permissions.set(String::new());
                    refresh();
                }
                Err(e) => create_role_error.set(Some(e)),
            }
        });
    };

    rsx! {
        div { class: "page-head",
            div {
                div { class: "page-title", "Users & Roles" }
                div { class: "page-sub", "Manage accounts, role assignment, and the permission catalog" }
            }
        }

        if !has_users_manage {
            div { class: "form-error", "You don't have the users.manage permission — viewing is read-only." }
        }
        if let Some(err) = load_error() {
            div { class: "form-error", "{err}" }
        }

        section { class: "block",
            span { class: "label", "Users" }
            div { class: "table-wrap",
                table {
                    thead { tr { th { "Username" } th { "Roles" } th { "Created" } } }
                    tbody {
                        for u in users() {
                            tr { key: "{u.id}",
                                td { b { "{u.username}" } }
                                td {
                                    for r in u.roles.iter() {
                                        span { class: "pill pill-blue", style: "margin-right:4px;", "{r}" }
                                    }
                                }
                                td { class: "mono", "{u.created_at}" }
                            }
                        }
                    }
                }
            }
        }

        if has_users_manage {
            section { class: "block card",
                span { class: "label", "Create User" }
                form { onsubmit: on_create_user,
                    div { class: "field-row grid-3", style: "margin-bottom:10px;",
                        div { class: "field",
                            label { "Username" }
                            input { class: "finput", value: "{new_username}", oninput: move |e| new_username.set(e.value()) }
                        }
                        div { class: "field",
                            label { "Password" }
                            input { class: "finput", r#type: "password", value: "{new_password}", oninput: move |e| new_password.set(e.value()) }
                        }
                        div { class: "field",
                            label { "Roles (comma separated)" }
                            input { class: "finput", placeholder: "trader,viewer", value: "{new_roles}", oninput: move |e| new_roles.set(e.value()) }
                        }
                    }
                    if let Some(err) = create_user_error() {
                        div { class: "form-error", "{err}" }
                    }
                    button { class: "btn btn-primary", r#type: "submit", "Create user" }
                }
            }
        }

        section { class: "block",
            span { class: "label", "Roles & Permissions" }
            div { class: "field-row grid-3",
                for r in roles() {
                    div { class: "card", key: "{r.name}",
                        div { style: "display:flex; justify-content:space-between; align-items:center; margin-bottom:10px;",
                            span { class: "pill pill-purple", "{r.name}" }
                        }
                        div { style: "display:flex; flex-wrap:wrap; gap:6px;",
                            if r.permissions.is_empty() {
                                span { class: "mono", style: "font-size:11px; color:var(--muted2);", "read-only — no grants" }
                            }
                            for p in r.permissions.iter() {
                                span { class: "pill pill-muted", "{p}" }
                            }
                        }
                    }
                }
            }
        }

        if has_roles_manage {
            section { class: "block card",
                span { class: "label", "Create Role" }
                form { onsubmit: on_create_role,
                    div { class: "field-row grid-2", style: "margin-bottom:10px;",
                        div { class: "field",
                            label { "Name" }
                            input { class: "finput", value: "{new_role_name}", oninput: move |e| new_role_name.set(e.value()) }
                        }
                        div { class: "field",
                            label { "Permissions (comma separated)" }
                            input {
                                class: "finput",
                                placeholder: permissions().join(", "),
                                value: "{new_role_permissions}",
                                oninput: move |e| new_role_permissions.set(e.value()),
                            }
                        }
                    }
                    if let Some(err) = create_role_error() {
                        div { class: "form-error", "{err}" }
                    }
                    button { class: "btn btn-primary", r#type: "submit", "Create role" }
                }
            }
        }
    }
}
