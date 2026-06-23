//! Pure store for the engine's exchange/pair registry
//! (`GET /v1/exchanges`, `GET /v1/exchanges/{name}/pairs`). Replaces
//! hardcoding exchange names in the UI — per ROADMAP 1b, "no exchange
//! name or trading pair belongs in code; the database is the only source
//! of truth," and that applies to this UI exactly as much as it does to
//! the engine.

use std::collections::HashMap;

use crate::api::{ExchangeResponse, PairResponse};

#[derive(Debug, Clone, Default)]
pub struct ExchangeCatalog {
    exchanges: Vec<ExchangeResponse>,
    pairs: HashMap<String, Vec<PairResponse>>,
}

impl ExchangeCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_exchanges(&mut self, exchanges: Vec<ExchangeResponse>) {
        self.exchanges = exchanges;
    }

    pub fn set_pairs(&mut self, exchange: &str, pairs: Vec<PairResponse>) {
        self.pairs.insert(exchange.to_string(), pairs);
    }

    pub fn exchanges(&self) -> &[ExchangeResponse] {
        &self.exchanges
    }

    pub fn pairs_for(&self, exchange: &str) -> &[PairResponse] {
        self.pairs.get(exchange).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Flattens every exchange's pairs into `(exchange_name, "BASE/QUOTE")`
    /// rows — the data source for the chart page's symbol search combobox,
    /// which lets a user filter across every exchange at once instead of
    /// picking an exchange first.
    pub fn symbol_options(&self) -> Vec<(String, String)> {
        self.exchanges
            .iter()
            .flat_map(|exchange| {
                self.pairs_for(&exchange.name).iter().map(|pair| {
                    (
                        exchange.name.clone(),
                        format!("{}/{}", pair.base, pair.quote),
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(name: &str) -> ExchangeResponse {
        ExchangeResponse {
            name: name.to_string(),
            display_name: name.to_string(),
            enabled: true,
        }
    }

    fn pair(base: &str, quote: &str) -> PairResponse {
        PairResponse {
            base: base.to_string(),
            quote: quote.to_string(),
            market_type: "spot".to_string(),
            active: true,
        }
    }

    #[test]
    fn new_catalog_has_no_exchanges() {
        let catalog = ExchangeCatalog::new();
        assert!(catalog.exchanges().is_empty());
    }

    #[test]
    fn set_exchanges_replaces_the_list() {
        let mut catalog = ExchangeCatalog::new();
        catalog.set_exchanges(vec![exchange("tabdeal"), exchange("hitobit")]);
        assert_eq!(catalog.exchanges().len(), 2);
    }

    #[test]
    fn pairs_for_unknown_exchange_returns_empty() {
        let catalog = ExchangeCatalog::new();
        assert!(catalog.pairs_for("tabdeal").is_empty());
    }

    #[test]
    fn set_pairs_stores_pairs_for_that_exchange_only() {
        let mut catalog = ExchangeCatalog::new();
        catalog.set_pairs("tabdeal", vec![pair("USDT", "IRT")]);

        assert_eq!(catalog.pairs_for("tabdeal").len(), 1);
        assert!(catalog.pairs_for("hitobit").is_empty());
    }

    #[test]
    fn set_pairs_overwrites_previous_pairs_for_the_same_exchange() {
        let mut catalog = ExchangeCatalog::new();
        catalog.set_pairs("tabdeal", vec![pair("USDT", "IRT")]);
        catalog.set_pairs("tabdeal", vec![pair("BTC", "IRT"), pair("USDT", "IRT")]);

        assert_eq!(catalog.pairs_for("tabdeal").len(), 2);
    }

    #[test]
    fn symbol_options_is_empty_when_no_exchanges() {
        let catalog = ExchangeCatalog::new();
        assert!(catalog.symbol_options().is_empty());
    }

    #[test]
    fn symbol_options_flattens_pairs_across_all_exchanges() {
        let mut catalog = ExchangeCatalog::new();
        catalog.set_exchanges(vec![exchange("tabdeal"), exchange("hitobit")]);
        catalog.set_pairs("tabdeal", vec![pair("USDT", "IRT"), pair("BTC", "IRT")]);
        catalog.set_pairs("hitobit", vec![pair("USDT", "IRT")]);

        let options = catalog.symbol_options();

        assert_eq!(options.len(), 3);
        assert!(options.contains(&("tabdeal".to_string(), "USDT/IRT".to_string())));
        assert!(options.contains(&("tabdeal".to_string(), "BTC/IRT".to_string())));
        assert!(options.contains(&("hitobit".to_string(), "USDT/IRT".to_string())));
    }
}
