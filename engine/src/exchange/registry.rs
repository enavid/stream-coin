use crate::price::entity::MarketType;

#[derive(Debug, Clone)]
pub struct ExchangeRecord {
    pub name: String,
    pub display_name: String,
    pub ws_url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct TradingPairRecord {
    pub exchange_name: String,
    pub base: String,
    pub quote: String,
    pub market_type: MarketType,
    pub active: bool,
}

#[derive(Default)]
pub struct ExchangeRegistry {
    exchanges: Vec<ExchangeRecord>,
    pairs: Vec<TradingPairRecord>,
}

impl ExchangeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_exchange(&mut self, record: ExchangeRecord) {
        self.exchanges.push(record);
    }

    pub fn add_pair(&mut self, record: TradingPairRecord) {
        self.pairs.push(record);
    }

    /// Inserts or updates a pair, keyed by `(exchange_name, base, quote, market_type)`.
    /// Idempotent — used by the top-market seeder so re-running it doesn't
    /// duplicate entries in the in-memory registry.
    pub fn upsert_pair(&mut self, record: TradingPairRecord) {
        match self.pairs.iter_mut().find(|p| {
            p.exchange_name == record.exchange_name
                && p.base == record.base
                && p.quote == record.quote
                && p.market_type == record.market_type
        }) {
            Some(existing) => *existing = record,
            None => self.pairs.push(record),
        }
    }

    pub fn get_enabled_exchanges(&self) -> Vec<&ExchangeRecord> {
        self.exchanges.iter().filter(|e| e.enabled).collect()
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.exchanges.iter().any(|e| e.name == name && e.enabled)
    }

    pub fn find_ws_url(&self, name: &str) -> Option<&str> {
        self.exchanges
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.ws_url.as_str())
    }

    /// Returns `true` if the exchange was found and updated.
    pub fn enable(&mut self, name: &str) -> bool {
        if let Some(e) = self.exchanges.iter_mut().find(|e| e.name == name) {
            e.enabled = true;
            true
        } else {
            false
        }
    }

    /// Returns `true` if the exchange was found and updated.
    pub fn disable(&mut self, name: &str) -> bool {
        if let Some(e) = self.exchanges.iter_mut().find(|e| e.name == name) {
            e.enabled = false;
            true
        } else {
            false
        }
    }

    pub fn get_active_pairs(
        &self,
        exchange_name: &str,
        market_type_filter: Option<&MarketType>,
    ) -> Vec<&TradingPairRecord> {
        self.pairs
            .iter()
            .filter(|p| {
                p.exchange_name == exchange_name
                    && p.active
                    && market_type_filter
                        .map(|mt| &p.market_type == mt)
                        .unwrap_or(true)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabdeal(enabled: bool) -> ExchangeRecord {
        ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled,
        }
    }

    fn hitobit(enabled: bool) -> ExchangeRecord {
        ExchangeRecord {
            name: "hitobit".to_string(),
            display_name: "Hitobit".to_string(),
            ws_url: "wss://hitobit.example.com".to_string(),
            enabled,
        }
    }

    #[test]
    fn startup_loads_only_enabled_exchanges() {
        let mut registry = ExchangeRegistry::new();
        registry.add_exchange(tabdeal(true));
        registry.add_exchange(hitobit(false));

        let enabled = registry.get_enabled_exchanges();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "tabdeal");
    }

    #[test]
    fn enable_sets_exchange_as_enabled() {
        let mut registry = ExchangeRegistry::new();
        registry.add_exchange(hitobit(false));

        let changed = registry.enable("hitobit");
        assert!(changed, "enable must return true when exchange is found");
        assert!(registry.is_enabled("hitobit"));
    }

    #[test]
    fn disable_sets_exchange_as_disabled() {
        let mut registry = ExchangeRegistry::new();
        registry.add_exchange(tabdeal(true));

        let changed = registry.disable("tabdeal");
        assert!(changed);
        assert!(!registry.is_enabled("tabdeal"));
    }

    #[test]
    fn enable_returns_false_when_exchange_not_found() {
        let mut registry = ExchangeRegistry::new();
        assert!(!registry.enable("unknown"));
    }

    #[test]
    fn get_active_pairs_returns_only_active() {
        let mut registry = ExchangeRegistry::new();
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "USDT".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: true,
        });
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "BTC".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: false,
        });

        let pairs = registry.get_active_pairs("tabdeal", None);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].base, "USDT");
    }

    #[test]
    fn get_active_pairs_filters_by_market_type() {
        let mut registry = ExchangeRegistry::new();
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "USDT".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: true,
        });
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "BTC".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Futures,
            active: true,
        });

        let spot_pairs = registry.get_active_pairs("tabdeal", Some(&MarketType::Spot));
        assert_eq!(spot_pairs.len(), 1);
        assert_eq!(spot_pairs[0].base, "USDT");

        let futures_pairs = registry.get_active_pairs("tabdeal", Some(&MarketType::Futures));
        assert_eq!(futures_pairs.len(), 1);
        assert_eq!(futures_pairs[0].base, "BTC");
    }

    fn pair(exchange: &str, base: &str, quote: &str, active: bool) -> TradingPairRecord {
        TradingPairRecord {
            exchange_name: exchange.to_string(),
            base: base.to_string(),
            quote: quote.to_string(),
            market_type: MarketType::Spot,
            active,
        }
    }

    #[test]
    fn upsert_pair_inserts_when_not_present() {
        let mut registry = ExchangeRegistry::new();
        registry.upsert_pair(pair("coinex", "BTC", "USDT", true));
        assert_eq!(registry.get_active_pairs("coinex", None).len(), 1);
    }

    #[test]
    fn upsert_pair_is_idempotent_on_duplicate_key() {
        let mut registry = ExchangeRegistry::new();
        registry.upsert_pair(pair("coinex", "BTC", "USDT", true));
        registry.upsert_pair(pair("coinex", "BTC", "USDT", true));
        assert_eq!(
            registry.get_active_pairs("coinex", None).len(),
            1,
            "re-upserting the same key must not duplicate"
        );
    }

    #[test]
    fn upsert_pair_updates_active_flag_on_matching_key() {
        let mut registry = ExchangeRegistry::new();
        registry.upsert_pair(pair("coinex", "BTC", "USDT", false));
        registry.upsert_pair(pair("coinex", "BTC", "USDT", true));

        let pairs = registry.get_active_pairs("coinex", None);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].active);
    }

    #[test]
    fn upsert_pair_keeps_distinct_exchanges_separate() {
        let mut registry = ExchangeRegistry::new();
        registry.upsert_pair(pair("coinex", "BTC", "USDT", true));
        registry.upsert_pair(pair("tabdeal", "BTC", "USDT", true));

        assert_eq!(registry.get_active_pairs("coinex", None).len(), 1);
        assert_eq!(registry.get_active_pairs("tabdeal", None).len(), 1);
    }
}
