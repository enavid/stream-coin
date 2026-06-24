use std::fmt;

use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Deserialize, Debug)]
pub struct ApiSuccess<T> {
    #[allow(dead_code)]
    pub success: bool,
    #[allow(dead_code)]
    pub message: String,
    pub data: T,
}

#[derive(Deserialize, Debug)]
pub struct ApiError {
    #[allow(dead_code)]
    pub success: bool,
    pub message: String,
    pub errors: Vec<FieldError>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.errors.is_empty() {
            write!(f, "{}", self.message)
        } else {
            let details: Vec<String> = self
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect();
            write!(f, "{} ({})", self.message, details.join(", "))
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct TickerData {
    pub exchange: String,
    pub pair: String,
}

#[derive(Deserialize, Debug)]
pub struct TickerListData {
    pub tickers: Vec<TickerData>,
}

#[derive(Deserialize, Debug)]
pub struct BackfillData {
    pub candles_written: usize,
}

#[derive(Deserialize, Debug)]
pub struct SeedPairsData {
    pub pairs_seeded: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success_json() -> &'static str {
        r#"{"success":true,"message":"Ticker started","data":{"exchange":"tabdeal","pair":"USDT/IRT"}}"#
    }

    fn error_json() -> &'static str {
        r#"{"success":false,"message":"Validation failed","errors":[{"field":"symbol","message":"must be BASE/QUOTE format"}]}"#
    }

    #[test]
    fn api_success_deserializes_ticker_data() {
        let result: ApiSuccess<TickerData> = serde_json::from_str(success_json()).unwrap();
        assert!(result.success);
    }

    #[test]
    fn api_success_data_exchange_matches() {
        let result: ApiSuccess<TickerData> = serde_json::from_str(success_json()).unwrap();
        assert_eq!(result.data.exchange, "tabdeal");
    }

    #[test]
    fn api_success_data_pair_matches() {
        let result: ApiSuccess<TickerData> = serde_json::from_str(success_json()).unwrap();
        assert_eq!(result.data.pair, "USDT/IRT");
    }

    #[test]
    fn api_error_deserializes_field_errors() {
        let result: ApiError = serde_json::from_str(error_json()).unwrap();
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn api_error_field_name_matches() {
        let result: ApiError = serde_json::from_str(error_json()).unwrap();
        assert_eq!(result.errors[0].field, "symbol");
    }

    #[test]
    fn ticker_list_deserializes_multiple_tickers() {
        let json = r#"{"success":true,"message":"ok","data":{"tickers":[{"exchange":"tabdeal","pair":"USDT/IRT"},{"exchange":"tabdeal","pair":"BTC/IRT"}]}}"#;
        let result: ApiSuccess<TickerListData> = serde_json::from_str(json).unwrap();
        assert_eq!(result.data.tickers.len(), 2);
    }

    #[test]
    fn ticker_list_empty_tickers_deserializes() {
        let json = r#"{"success":true,"message":"ok","data":{"tickers":[]}}"#;
        let result: ApiSuccess<TickerListData> = serde_json::from_str(json).unwrap();
        assert!(result.data.tickers.is_empty());
    }

    #[test]
    fn backfill_success_deserializes_candles_written_count() {
        let json =
            r#"{"success":true,"message":"Backfill complete","data":{"candles_written":42}}"#;
        let result: ApiSuccess<BackfillData> = serde_json::from_str(json).unwrap();
        assert_eq!(result.data.candles_written, 42);
    }

    #[test]
    fn backfill_error_response_deserializes_as_api_error() {
        let json = r#"{"success":false,"message":"exchange 'tabdeal' has no historical candle source","errors":[]}"#;
        let result: ApiError = serde_json::from_str(json).unwrap();
        assert!(result.message.contains("historical candle source"));
    }

    #[test]
    fn api_success_missing_data_field_returns_err() {
        let json = r#"{"success":true,"message":"ok"}"#;
        let result: Result<ApiSuccess<TickerData>, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "missing data field must fail deserialization"
        );
    }

    #[test]
    fn api_success_success_as_string_returns_err() {
        let json =
            r#"{"success":"yes","message":"ok","data":{"exchange":"tabdeal","pair":"USDT/IRT"}}"#;
        let result: Result<ApiSuccess<TickerData>, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "success as a string must fail deserialization"
        );
    }

    #[test]
    fn seed_pairs_success_deserializes_pairs_seeded_count() {
        let json = r#"{"success":true,"message":"Pairs seeded","data":{"pairs_seeded":20}}"#;
        let result: ApiSuccess<SeedPairsData> = serde_json::from_str(json).unwrap();
        assert_eq!(result.data.pairs_seeded, 20);
    }

    #[test]
    fn ticker_data_missing_pair_field_returns_err() {
        let json = r#"{"exchange":"tabdeal"}"#;
        let result: Result<TickerData, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "missing pair field must fail deserialization"
        );
    }
}
