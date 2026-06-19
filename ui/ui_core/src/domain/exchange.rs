/// Exchanges with a registered adapter in the engine.
/// Keep in sync with `engine/bin/http.rs` adapter registrations.
pub const SUPPORTED_EXCHANGES: &[&str] = &["tabdeal"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_exchanges_is_not_empty() {
        assert!(!SUPPORTED_EXCHANGES.is_empty());
    }

    #[test]
    fn supported_exchanges_contains_tabdeal() {
        assert!(
            SUPPORTED_EXCHANGES.contains(&"tabdeal"),
            "tabdeal must always be listed — it is the only registered adapter"
        );
    }

    #[test]
    fn supported_exchanges_only_contains_registered_adapters() {
        // Update this list when a new adapter is added to engine/bin/http.rs.
        const REGISTERED_IN_ENGINE: &[&str] = &["tabdeal"];
        for ex in SUPPORTED_EXCHANGES {
            assert!(
                REGISTERED_IN_ENGINE.contains(ex),
                "'{ex}' is in SUPPORTED_EXCHANGES but has no registered engine adapter"
            );
        }
    }
}
