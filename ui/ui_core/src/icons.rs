//! Hand-rolled inline-SVG icon set — no icon font, no network fetch, no
//! extra dependency. Every icon is `18x18`, `viewBox="0 0 24 24"`,
//! `stroke="currentColor"` so it inherits the surrounding text color (the
//! same convention the mockup's CSS already assumes for nav links/pills).
//! Presentational only — no test value, same as `TickerCard`
//! which also has none.

use dioxus::prelude::*;

const STROKE: &str = "1.8";

#[component]
pub fn IconDashboard() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            rect { x: "3", y: "3", width: "8", height: "8", rx: "2" }
            rect { x: "13", y: "3", width: "8", height: "8", rx: "2" }
            rect { x: "3", y: "13", width: "8", height: "8", rx: "2" }
            rect { x: "13", y: "13", width: "8", height: "8", rx: "2" }
        }
    }
}

#[component]
pub fn IconStrategy() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            rect { x: "7", y: "7", width: "10", height: "10", rx: "2" }
            line { x1: "12", y1: "2", x2: "12", y2: "7" }
            line { x1: "12", y1: "17", x2: "12", y2: "22" }
            line { x1: "2", y1: "12", x2: "7", y2: "12" }
            line { x1: "17", y1: "12", x2: "22", y2: "12" }
        }
    }
}

#[component]
pub fn IconChart() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            line { x1: "5", y1: "3", x2: "5", y2: "21" }
            line { x1: "5", y1: "21", x2: "21", y2: "21" }
            line { x1: "8", y1: "8", x2: "8", y2: "14" }
            line { x1: "8", y1: "6", x2: "8", y2: "8" }
            line { x1: "8", y1: "14", x2: "8", y2: "16" }
            line { x1: "13", y1: "11", x2: "13", y2: "17" }
            line { x1: "13", y1: "9", x2: "13", y2: "11" }
            line { x1: "13", y1: "17", x2: "13", y2: "19" }
            line { x1: "18", y1: "7", x2: "18", y2: "12" }
            line { x1: "18", y1: "5", x2: "18", y2: "7" }
            line { x1: "18", y1: "12", x2: "18", y2: "14" }
        }
    }
}

#[component]
pub fn IconBacktest() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            circle { cx: "12", cy: "13", r: "8" }
            polyline { points: "12,9 12,13 15,15" }
            polyline { points: "5,3 5,7 9,7" }
        }
    }
}

#[component]
pub fn IconOrders() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            line { x1: "8", y1: "6", x2: "21", y2: "6" }
            line { x1: "8", y1: "12", x2: "21", y2: "12" }
            line { x1: "8", y1: "18", x2: "21", y2: "18" }
            circle { cx: "3.5", cy: "6", r: "1.3", fill: "currentColor", stroke: "none" }
            circle { cx: "3.5", cy: "12", r: "1.3", fill: "currentColor", stroke: "none" }
            circle { cx: "3.5", cy: "18", r: "1.3", fill: "currentColor", stroke: "none" }
        }
    }
}

#[component]
pub fn IconAdmin() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            circle { cx: "9", cy: "8", r: "3" }
            path { d: "M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6" }
            circle { cx: "17", cy: "8", r: "2.2" }
            path { d: "M16 14.2c2.3.4 4 2.5 4 5" }
        }
    }
}

#[component]
pub fn IconSettings() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            line { x1: "5", y1: "3", x2: "5", y2: "21" }
            circle { cx: "5", cy: "9", r: "2.2", fill: "currentColor", stroke: "none" }
            line { x1: "12", y1: "3", x2: "12", y2: "21" }
            circle { cx: "12", cy: "16", r: "2.2", fill: "currentColor", stroke: "none" }
            line { x1: "19", y1: "3", x2: "19", y2: "21" }
            circle { cx: "19", cy: "6", r: "2.2", fill: "currentColor", stroke: "none" }
        }
    }
}

#[component]
pub fn IconLogout() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            path { d: "M9 4H5a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h4" }
            polyline { points: "15,16 20,11 15,6" }
            line { x1: "20", y1: "11", x2: "9", y2: "11" }
        }
    }
}

#[component]
pub fn IconMenu() -> Element {
    rsx! {
        svg { width: "20", height: "20", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            line { x1: "3", y1: "6", x2: "21", y2: "6" }
            line { x1: "3", y1: "12", x2: "21", y2: "12" }
            line { x1: "3", y1: "18", x2: "21", y2: "18" }
        }
    }
}

#[component]
pub fn IconClose() -> Element {
    rsx! {
        svg { width: "18", height: "18", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            line { x1: "5", y1: "5", x2: "19", y2: "19" }
            line { x1: "19", y1: "5", x2: "5", y2: "19" }
        }
    }
}

#[component]
pub fn IconPlus() -> Element {
    rsx! {
        svg { width: "14", height: "14", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2.4",
            line { x1: "12", y1: "4", x2: "12", y2: "20" }
            line { x1: "4", y1: "12", x2: "20", y2: "12" }
        }
    }
}

/// Sidebar collapse/expand toggle — a panel outline with a divider near
/// the left edge, the same glyph convention used by VS Code/Linear/Notion
/// for "toggle sidebar".
#[component]
pub fn IconPanelLeft() -> Element {
    rsx! {
        svg { width: "16", height: "16", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            rect { x: "3", y: "4", width: "18", height: "16", rx: "2" }
            line { x1: "9.5", y1: "4", x2: "9.5", y2: "20" }
        }
    }
}

#[component]
pub fn IconChevronDown() -> Element {
    rsx! {
        svg { width: "12", height: "12", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            polyline { points: "5,8 12,16 19,8" }
        }
    }
}

/// Shown when the current theme is dark — clicking switches to light.
#[component]
pub fn IconSun() -> Element {
    rsx! {
        svg { width: "16", height: "16", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            circle { cx: "12", cy: "12", r: "4.5" }
            line { x1: "12", y1: "2", x2: "12", y2: "5" }
            line { x1: "12", y1: "19", x2: "12", y2: "22" }
            line { x1: "2", y1: "12", x2: "5", y2: "12" }
            line { x1: "19", y1: "12", x2: "22", y2: "12" }
            line { x1: "4.2", y1: "4.2", x2: "6.3", y2: "6.3" }
            line { x1: "17.7", y1: "17.7", x2: "19.8", y2: "19.8" }
            line { x1: "4.2", y1: "19.8", x2: "6.3", y2: "17.7" }
            line { x1: "17.7", y1: "6.3", x2: "19.8", y2: "4.2" }
        }
    }
}

/// Shown when the current theme is light — clicking switches to dark.
#[component]
pub fn IconMoon() -> Element {
    rsx! {
        svg { width: "16", height: "16", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: STROKE,
            path { d: "M20 14.5A8.5 8.5 0 1 1 9.5 4a7 7 0 0 0 10.5 10.5z" }
        }
    }
}
