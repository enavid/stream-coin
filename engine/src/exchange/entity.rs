use std::fmt;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExchangeId(String);

impl serde::Serialize for ExchangeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExchangeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.trim().is_empty() {
            return Err(de::Error::custom("exchange name cannot be empty"));
        }
        Ok(ExchangeId::new(&s))
    }
}

impl ExchangeId {
    pub fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn try_new(name: &str) -> Result<Self, &'static str> {
        if name.trim().is_empty() {
            return Err("exchange name cannot be empty");
        }
        Ok(Self::new(name))
    }
}

impl fmt::Display for ExchangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_id_display_is_lowercase_name() {
        let id = ExchangeId::new("Tabdeal");
        assert_eq!(id.to_string(), "tabdeal");
    }

    #[test]
    fn exchange_id_equality() {
        let a = ExchangeId::new("tabdeal");
        let b = ExchangeId::new("Tabdeal");
        assert_eq!(a, b);
    }

    #[test]
    fn exchange_id_rejects_empty_name() {
        let result = ExchangeId::try_new("");
        assert!(result.is_err());
    }

    #[test]
    fn exchange_id_deserialize_accepts_lowercase_name() {
        let id: ExchangeId = serde_json::from_str(r#""tabdeal""#).unwrap();
        assert_eq!(id.to_string(), "tabdeal");
    }

    #[test]
    fn exchange_id_deserialize_accepts_uppercase_and_normalizes() {
        let id: ExchangeId = serde_json::from_str(r#""TABDEAL""#).unwrap();
        assert_eq!(id.to_string(), "tabdeal");
    }

    #[test]
    fn exchange_id_deserialize_rejects_empty_string() {
        let result: Result<ExchangeId, _> = serde_json::from_str(r#""""#);
        assert!(result.is_err(), "empty string must be rejected");
    }

    #[test]
    fn exchange_id_deserialize_rejects_whitespace_only() {
        let result: Result<ExchangeId, _> = serde_json::from_str(r#""   ""#);
        assert!(result.is_err(), "whitespace-only string must be rejected");
    }
}
