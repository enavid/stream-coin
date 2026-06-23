use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::candle::entity::{Candle, Interval};
use crate::exchange::entity::ExchangeId;
use crate::exchange::historical_port::{HistoricalCandleSource, HistoricalCandleSourceError};
use crate::price::entity::TradingPair;

const BASE_URL: &str = "https://api.coinex.com/v2";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_LIMIT: u32 = 1000;

/// Maps the engine's `Interval` to CoinEx's `period` query parameter.
/// CoinEx supports a wider set of periods; only the four this engine
/// aggregates are mapped.
pub fn period_for_interval(interval: Interval) -> &'static str {
    match interval {
        Interval::OneMinute => "1min",
        Interval::FiveMinutes => "5min",
        Interval::FifteenMinutes => "15min",
        Interval::OneHour => "1hour",
    }
}

/// Parses a single kline item from CoinEx's `GET /spot/kline` response into
/// a `Candle`. CoinEx returns OHLCV fields as decimal strings; per the
/// project's Financial Precision rule, these are truncated to minor-unit
/// `u64` at the point of ingestion, never parsed as `f64`.
fn parse_kline_item(market: &str, interval: Interval, item: &Value) -> Result<Candle, String> {
    let created_at_ms = item["created_at"]
        .as_i64()
        .ok_or_else(|| "missing created_at field".to_string())?;
    let time = DateTime::<Utc>::from_timestamp_millis(created_at_ms)
        .ok_or_else(|| format!("invalid created_at timestamp: {created_at_ms}"))?;

    let open = super::parse_minor_units(field_str(item, "open")?)?;
    let high = super::parse_minor_units(field_str(item, "high")?)?;
    let low = super::parse_minor_units(field_str(item, "low")?)?;
    let close = super::parse_minor_units(field_str(item, "close")?)?;
    let volume = super::parse_minor_units(field_str(item, "volume")?)?;

    let pair = super::market_to_pair(market);

    Ok(Candle {
        exchange: "coinex".to_string(),
        pair: format!("{}/{}", pair.base, pair.quote),
        interval,
        time,
        open,
        high,
        low,
        close,
        volume,
    })
}

fn field_str<'a>(item: &'a Value, name: &str) -> Result<&'a str, String> {
    item[name]
        .as_str()
        .ok_or_else(|| format!("missing {name} field"))
}

/// Parses a full `GET /spot/kline` response body (`{"code":0,"data":[...]}`)
/// into candles. A malformed envelope or any single bad item is a permanent
/// `Serialization` error — CoinEx will not return a different shape on retry.
fn parse_kline_page(
    market: &str,
    interval: Interval,
    body: &str,
) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| HistoricalCandleSourceError::Serialization(format!("invalid json: {e}")))?;

    let data = v["data"]
        .as_array()
        .ok_or_else(|| HistoricalCandleSourceError::Serialization("missing data array".into()))?;

    data.iter()
        .map(|item| {
            parse_kline_item(market, interval, item)
                .map_err(HistoricalCandleSourceError::Serialization)
        })
        .collect()
}

/// Classifies an HTTP response status per the project's transient/permanent
/// rule. Returns `None` for 2xx (success — caller proceeds to parse the body).
fn classify_http_status(status: u16, body: &str) -> Option<HistoricalCandleSourceError> {
    match super::classify_http_status(status, body) {
        super::HttpStatusClass::Success => None,
        super::HttpStatusClass::Transient { status, body } => {
            Some(HistoricalCandleSourceError::ServerError { status, body })
        }
        super::HttpStatusClass::Permanent { status, body } => {
            Some(HistoricalCandleSourceError::ClientError { status, body })
        }
    }
}

/// Abstraction over fetching one page of klines, so the pagination algorithm
/// (`paginate`) can be unit tested with a fake in-memory page source instead
/// of a real network call.
#[async_trait]
trait KlinePageFetcher: Send + Sync {
    async fn fetch_page(
        &self,
        market: &str,
        period: &str,
        start_ms: i64,
        end_ms: i64,
        limit: u32,
    ) -> Result<Vec<Candle>, HistoricalCandleSourceError>;
}

/// Repeatedly calls `fetcher` advancing `start_ms` past the last candle
/// returned, until the range is exhausted, a page comes back empty, or a
/// page fails to advance the cursor (defensive — avoids an infinite loop on
/// a misbehaving page source).
async fn paginate<F: KlinePageFetcher>(
    fetcher: &F,
    market: &str,
    period: &str,
    interval: Interval,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
    let mut start_ms = from.timestamp_millis();
    let end_ms = to.timestamp_millis();
    let interval_ms = interval.as_secs() as i64 * 1000;
    let mut all = Vec::new();

    loop {
        if start_ms > end_ms {
            break;
        }

        let page = fetcher
            .fetch_page(market, period, start_ms, end_ms, PAGE_LIMIT)
            .await?;

        if page.is_empty() {
            tracing::debug!(
                market,
                start_ms,
                end_ms,
                "coinex kline page empty, stopping pagination"
            );
            break;
        }

        let page_len = page.len();
        let last_time_ms = page
            .iter()
            .map(|c| c.time.timestamp_millis())
            .max()
            .unwrap_or(start_ms);
        all.extend(page);

        let next_start_ms = last_time_ms + interval_ms;
        tracing::debug!(
            market,
            page_len,
            start_ms,
            next_start_ms,
            "coinex kline page fetched"
        );

        if next_start_ms <= start_ms {
            tracing::warn!(
                market,
                start_ms,
                "coinex kline page did not advance cursor, stopping pagination defensively"
            );
            break;
        }
        start_ms = next_start_ms;
    }

    Ok(all)
}

/// Historical REST source for CoinEx — `GET /spot/kline`. Public market data
/// endpoint; no API key required.
pub struct CoinexHistoricalAdapter {
    base_url: String,
    http_client: reqwest::Client,
}

impl CoinexHistoricalAdapter {
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client: reqwest::Client::new(),
        }
    }
}

impl Default for CoinexHistoricalAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KlinePageFetcher for CoinexHistoricalAdapter {
    async fn fetch_page(
        &self,
        market: &str,
        period: &str,
        start_ms: i64,
        end_ms: i64,
        limit: u32,
    ) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
        let url = format!(
            "{}/spot/kline?market={market}&period={period}&start_time={start_ms}&end_time={end_ms}&limit={limit}",
            self.base_url
        );

        let response = tokio::time::timeout(REQUEST_TIMEOUT, self.http_client.get(&url).send())
            .await
            .map_err(|_| {
                tracing::error!(market, %url, "coinex kline request timed out");
                HistoricalCandleSourceError::NetworkTimeout("fetch_klines timed out".to_string())
            })?
            .map_err(|e| {
                tracing::error!(market, error = %e, "coinex kline network error");
                HistoricalCandleSourceError::NetworkTimeout(e.to_string())
            })?;

        let status = response.status().as_u16();
        let body = response.text().await.map_err(|e| {
            HistoricalCandleSourceError::Serialization(format!("failed to read response body: {e}"))
        })?;

        if let Some(err) = classify_http_status(status, &body) {
            tracing::warn!(
                market,
                status,
                transient = err.is_transient(),
                "coinex kline request failed"
            );
            return Err(err);
        }

        let interval = match period {
            "1min" => Interval::OneMinute,
            "5min" => Interval::FiveMinutes,
            "15min" => Interval::FifteenMinutes,
            _ => Interval::OneHour,
        };
        parse_kline_page(market, interval, &body)
    }
}

#[async_trait]
impl HistoricalCandleSource for CoinexHistoricalAdapter {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("coinex")
    }

    async fn fetch_klines(
        &self,
        pair: &TradingPair,
        interval: Interval,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
        let market = format!("{}{}", pair.base, pair.quote);
        let period = period_for_interval(interval);

        tracing::info!(
            exchange = "coinex",
            market = %market,
            interval = interval.as_str(),
            from = %from,
            to = %to,
            "fetching historical klines from coinex"
        );

        let result = paginate(self, &market, period, interval, from, to).await;

        match &result {
            Ok(candles) => tracing::info!(
                exchange = "coinex",
                market = %market,
                candle_count = candles.len(),
                "historical kline fetch complete"
            ),
            Err(e) => tracing::error!(
                exchange = "coinex",
                market = %market,
                error = %e,
                transient = e.is_transient(),
                "historical kline fetch failed"
            ),
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    // --- period_for_interval ---

    #[test]
    fn period_for_interval_maps_all_four_supported_intervals() {
        assert_eq!(period_for_interval(Interval::OneMinute), "1min");
        assert_eq!(period_for_interval(Interval::FiveMinutes), "5min");
        assert_eq!(period_for_interval(Interval::FifteenMinutes), "15min");
        assert_eq!(period_for_interval(Interval::OneHour), "1hour");
    }

    // --- parse_kline_page ---

    fn kline_body(items: Vec<Value>) -> String {
        json!({ "code": 0, "data": items, "message": "OK" }).to_string()
    }

    fn kline_item(
        created_at_ms: i64,
        open: &str,
        high: &str,
        low: &str,
        close: &str,
        volume: &str,
    ) -> Value {
        json!({
            "market": "BTCUSDT",
            "created_at": created_at_ms,
            "open": open,
            "high": high,
            "low": low,
            "close": close,
            "volume": volume,
            "value": "0"
        })
    }

    #[test]
    fn coinex_kline_response_parses_string_prices_to_minor_units() {
        let body = kline_body(vec![kline_item(
            1_689_920_330_000,
            "30000.00",
            "30050.00",
            "29990.00",
            "30010.00",
            "12.5",
        )]);
        let candles = parse_kline_page("BTCUSDT", Interval::OneHour, &body).unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, 30000);
        assert_eq!(candles[0].high, 30050);
        assert_eq!(candles[0].low, 29990);
        assert_eq!(candles[0].close, 30010);
        assert_eq!(candles[0].volume, 12);
    }

    #[test]
    fn coinex_kline_response_extracts_trading_pair_from_market() {
        let body = kline_body(vec![kline_item(
            1_689_920_330_000,
            "30000",
            "30000",
            "30000",
            "30000",
            "1",
        )]);
        let candles = parse_kline_page("BTCUSDT", Interval::OneHour, &body).unwrap();
        assert_eq!(candles[0].pair, "BTC/USDT");
        assert_eq!(candles[0].exchange, "coinex");
    }

    #[test]
    fn coinex_kline_response_negative_price_returns_serialization_error() {
        let body = kline_body(vec![kline_item(
            1_689_920_330_000,
            "-1",
            "30000",
            "30000",
            "30000",
            "1",
        )]);
        let result = parse_kline_page("BTCUSDT", Interval::OneHour, &body);
        assert!(matches!(
            result,
            Err(HistoricalCandleSourceError::Serialization(_))
        ));
    }

    #[test]
    fn coinex_kline_response_missing_field_returns_serialization_error() {
        let body = json!({
            "code": 0,
            "data": [{ "market": "BTCUSDT", "created_at": 1_689_920_330_000i64 }],
        })
        .to_string();
        let result = parse_kline_page("BTCUSDT", Interval::OneHour, &body);
        assert!(matches!(
            result,
            Err(HistoricalCandleSourceError::Serialization(_))
        ));
    }

    #[test]
    fn coinex_kline_response_missing_data_array_returns_serialization_error() {
        let body = json!({ "code": 0, "message": "OK" }).to_string();
        let result = parse_kline_page("BTCUSDT", Interval::OneHour, &body);
        assert!(matches!(
            result,
            Err(HistoricalCandleSourceError::Serialization(_))
        ));
    }

    #[test]
    fn coinex_kline_response_empty_data_array_returns_empty_vec() {
        let body = kline_body(vec![]);
        let candles = parse_kline_page("BTCUSDT", Interval::OneHour, &body).unwrap();
        assert!(candles.is_empty());
    }

    // --- classify_http_status ---

    #[test]
    fn classify_http_status_2xx_returns_none() {
        assert!(classify_http_status(200, "{}").is_none());
    }

    #[test]
    fn classify_http_status_5xx_is_transient_server_error() {
        let err = classify_http_status(503, "unavailable").unwrap();
        assert!(err.is_transient());
        assert!(matches!(
            err,
            HistoricalCandleSourceError::ServerError { status: 503, .. }
        ));
    }

    #[test]
    fn classify_http_status_4xx_is_permanent_client_error() {
        let err = classify_http_status(400, "bad request").unwrap();
        assert!(!err.is_transient());
        assert!(matches!(
            err,
            HistoricalCandleSourceError::ClientError { status: 400, .. }
        ));
    }

    // --- paginate ---

    struct FakeKlinePageFetcher {
        pages: Mutex<Vec<Vec<Candle>>>,
        calls: AtomicUsize,
    }

    impl FakeKlinePageFetcher {
        fn new(pages: Vec<Vec<Candle>>) -> Self {
            Self {
                pages: Mutex::new(pages),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl KlinePageFetcher for FakeKlinePageFetcher {
        async fn fetch_page(
            &self,
            _market: &str,
            _period: &str,
            _start_ms: i64,
            _end_ms: i64,
            _limit: u32,
        ) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut pages = self.pages.lock().unwrap();
            Ok(if pages.is_empty() {
                Vec::new()
            } else {
                pages.remove(0)
            })
        }
    }

    fn fake_candle(time_ms: i64) -> Candle {
        Candle {
            exchange: "coinex".to_string(),
            pair: "BTC/USDT".to_string(),
            interval: Interval::OneHour,
            time: DateTime::<Utc>::from_timestamp_millis(time_ms).unwrap(),
            open: 1,
            high: 1,
            low: 1,
            close: 1,
            volume: 1,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_klines_paginates_when_range_exceeds_one_page() {
        let hour_ms = Interval::OneHour.as_secs() as i64 * 1000;
        let page1 = vec![fake_candle(0), fake_candle(hour_ms)];
        let page2 = vec![fake_candle(2 * hour_ms), fake_candle(3 * hour_ms)];
        let fetcher = FakeKlinePageFetcher::new(vec![page1, page2, Vec::new()]);

        let from = DateTime::<Utc>::from_timestamp_millis(0).unwrap();
        let to = DateTime::<Utc>::from_timestamp_millis(10 * hour_ms).unwrap();

        let result = paginate(&fetcher, "BTCUSDT", "1hour", Interval::OneHour, from, to)
            .await
            .unwrap();

        assert_eq!(result.len(), 4, "must combine candles across pages");
        assert!(
            fetcher.calls.load(Ordering::SeqCst) >= 2,
            "must call fetch_page more than once for a multi-page range"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_klines_stops_pagination_on_empty_page() {
        let hour_ms = Interval::OneHour.as_secs() as i64 * 1000;
        let fetcher = FakeKlinePageFetcher::new(vec![Vec::new()]);

        let from = DateTime::<Utc>::from_timestamp_millis(0).unwrap();
        let to = DateTime::<Utc>::from_timestamp_millis(100 * hour_ms).unwrap();

        let result = paginate(&fetcher, "BTCUSDT", "1hour", Interval::OneHour, from, to)
            .await
            .unwrap();

        assert!(result.is_empty());
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            1,
            "must stop after the first empty page, not loop forever"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_klines_stops_when_page_does_not_advance_cursor() {
        let from = DateTime::<Utc>::from_timestamp_millis(0).unwrap();
        let to = DateTime::<Utc>::from_timestamp_millis(1_000_000).unwrap();
        // A page whose candle is older than `from` would compute a next_start
        // that doesn't advance past start_ms — pagination must bail instead
        // of looping forever.
        let fetcher = FakeKlinePageFetcher::new(vec![vec![fake_candle(-1)], vec![fake_candle(-1)]]);

        let result = paginate(&fetcher, "BTCUSDT", "1hour", Interval::OneHour, from, to)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_klines_returns_error_when_page_fetch_fails() {
        struct FailingFetcher;
        #[async_trait]
        impl KlinePageFetcher for FailingFetcher {
            async fn fetch_page(
                &self,
                _market: &str,
                _period: &str,
                _start_ms: i64,
                _end_ms: i64,
                _limit: u32,
            ) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
                Err(HistoricalCandleSourceError::ClientError {
                    status: 400,
                    body: "bad market".to_string(),
                })
            }
        }

        let from = DateTime::<Utc>::from_timestamp_millis(0).unwrap();
        let to = DateTime::<Utc>::from_timestamp_millis(1_000_000).unwrap();
        let result = paginate(
            &FailingFetcher,
            "BTCUSDT",
            "1hour",
            Interval::OneHour,
            from,
            to,
        )
        .await;

        assert!(matches!(
            result,
            Err(HistoricalCandleSourceError::ClientError { status: 400, .. })
        ));
    }

    #[test]
    fn coinex_historical_adapter_exchange_id_is_coinex() {
        let adapter = CoinexHistoricalAdapter::new();
        assert_eq!(adapter.exchange_id().to_string(), "coinex");
    }
}
