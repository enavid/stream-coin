//! Shared, platform-agnostic UI for stream-coin: components, reactive
//! state, and the wire protocol shared with the backend's WebSocket feed.
//!
//! Each platform (web, and later desktop/mobile) is a thin binary crate
//! that depends on this crate. The platform crate's root component calls
//! [`state::provide_app_state`], spawns its own platform-specific
//! WebSocket connection to feed it, and renders [`Dashboard`]. No UI or
//! business logic should live in the platform crates — only transport.

pub mod api;
pub mod app_shell;
pub mod auth;
pub mod components;
pub mod dashboard;
pub mod domain;
pub mod icons;
pub mod pages;
pub mod protocol;
pub mod router;
pub mod state;
pub mod theme;

pub use app_shell::AppShell;
pub use dashboard::Dashboard;
pub use router::Route;
