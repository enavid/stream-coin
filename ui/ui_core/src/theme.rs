//! Light/dark theme preference. A pure enum with no Dioxus/DOM dependency
//! so it's unit testable on the host target — the platform layer
//! (`ui/web/src/browser.rs`) is the only thing that persists it to
//! `localStorage` or reads it back; `ui_core` just holds the value and
//! renders the `data-theme` attribute the CSS keys off (see `main.css`'s
//! `:root[data-theme="light"]` override block).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    /// Defaults to [`Theme::Dark`] for anything that isn't exactly
    /// `"light"` — a corrupted or stale `localStorage` value should never
    /// fail to render, just fall back silently.
    pub fn parse(s: &str) -> Self {
        match s {
            "light" => Theme::Light,
            _ => Theme::Dark,
        }
    }

    pub fn toggled(&self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_dark() {
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn as_str_and_parse_round_trip_for_dark() {
        assert_eq!(Theme::parse(Theme::Dark.as_str()), Theme::Dark);
    }

    #[test]
    fn as_str_and_parse_round_trip_for_light() {
        assert_eq!(Theme::parse(Theme::Light.as_str()), Theme::Light);
    }

    #[test]
    fn parse_defaults_to_dark_for_unknown_input() {
        assert_eq!(Theme::parse("solarized"), Theme::Dark);
        assert_eq!(Theme::parse(""), Theme::Dark);
    }

    #[test]
    fn toggled_flips_dark_to_light_and_back() {
        assert_eq!(Theme::Dark.toggled(), Theme::Light);
        assert_eq!(Theme::Light.toggled(), Theme::Dark);
    }
}
