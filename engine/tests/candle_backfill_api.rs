use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::candle::entity::{Candle, Interval};
use stream_coin::exchange::entity::ExchangeId;
use stream_coin::exchange::historical_port::{HistoricalCandleSource, HistoricalCandleSourceError};
use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::db::candle_repository::{CandleRepository, FakeCandleRepository};
use stream_coin::infrastructure::db::python_strategy_repository::{
    FakePythonStrategyRepository, PythonStrategyRecord,
};
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};
use stream_coin::price::entity::TradingPair;

/// Test double standing in for a real exchange's historical REST API.
/// Returns a fixed candle set or a fixed error, regardless of the requested
/// range — handler-level tests only need to exercise the wiring, not the
/// real pagination/parsing logic already covered by `coinex::historical_adapter`'s
/// own unit tests.
struct FakeHistoricalSource {
    result: Mutex<Option<Result<Vec<Candle>, HistoricalCandleSourceError>>>,
}

impl FakeHistoricalSource {
    fn with_candles(candles: Vec<Candle>) -> Self {
        Self {
            result: Mutex::new(Some(Ok(candles))),
        }
    }

    fn with_error(err: HistoricalCandleSourceError) -> Self {
        Self {
            result: Mutex::new(Some(Err(err))),
        }
    }
}

#[async_trait]
impl HistoricalCandleSource for FakeHistoricalSource {
    fn exchange_id(&self) -> ExchangeId {
        ExchangeId::new("coinex")
    }

    async fn fetch_klines(
        &self,
        _pair: &TradingPair,
        _interval: Interval,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
    ) -> Result<Vec<Candle>, HistoricalCandleSourceError> {
        self.result
            .lock()
            .await
            .take()
            .expect("fetch_klines called more than once in this test")
    }
}

fn sample_candles(pair: &str, closes: &[u64]) -> Vec<Candle> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &close)| Candle {
            exchange: "coinex".to_string(),
            pair: pair.to_string(),
            interval: Interval::OneHour,
            time: Utc
                .timestamp_opt(1_700_000_000 + i as i64 * 3600, 0)
                .unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
        })
        .collect()
}

fn build_state(
    historical_sources: HashMap<String, Arc<dyn HistoricalCandleSource>>,
    candle_repository: Option<Arc<dyn CandleRepository>>,
) -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository,
        historical_sources: Arc::new(historical_sources),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        subscription_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    })
}

fn backfill_request(exchange: &str, pair: &str) -> Value {
    json!({
        "exchange": exchange,
        "pair": pair,
        "interval": "1h",
        "from": "2023-07-21T00:00:00Z",
        "to": "2023-07-21T05:00:00Z",
    })
}

#[actix_web::test]
async fn backfill_returns_400_for_exchange_without_historical_source() {
    let state = build_state(
        HashMap::new(),
        Some(Arc::new(FakeCandleRepository::new(vec![]))),
    );
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("tabdeal", "BTC/USDT"))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "exchange with no registered HistoricalCandleSource must return 400"
    );
}

#[actix_web::test]
async fn backfill_returns_400_when_from_is_after_to() {
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(vec![])),
    );
    let state = build_state(sources, Some(Arc::new(FakeCandleRepository::new(vec![]))));
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&json!({
            "exchange": "coinex",
            "pair": "BTC/USDT",
            "interval": "1h",
            "from": "2023-07-21T05:00:00Z",
            "to": "2023-07-21T00:00:00Z",
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400, "from after to must be rejected");
}

#[actix_web::test]
async fn backfill_returns_400_for_malformed_pair() {
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(vec![])),
    );
    let state = build_state(sources, Some(Arc::new(FakeCandleRepository::new(vec![]))));
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTCUSDT"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 400, "pair without a '/' must be rejected");
}

#[actix_web::test]
async fn backfill_returns_503_when_no_candle_repository_configured() {
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(vec![])),
    );
    let state = build_state(sources, None);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTC/USDT"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 503);
}

#[actix_web::test]
async fn backfill_returns_503_when_source_fails_transiently() {
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_error(
            HistoricalCandleSourceError::ServerError {
                status: 503,
                body: "unavailable".to_string(),
            },
        )),
    );
    let state = build_state(sources, Some(Arc::new(FakeCandleRepository::new(vec![]))));
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTC/USDT"))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        503,
        "a transient upstream failure must surface as 503, not 400"
    );
}

#[actix_web::test]
async fn backfill_returns_400_when_source_fails_permanently() {
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_error(
            HistoricalCandleSourceError::ClientError {
                status: 400,
                body: "bad market".to_string(),
            },
        )),
    );
    let state = build_state(sources, Some(Arc::new(FakeCandleRepository::new(vec![]))));
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTC/USDT"))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        400,
        "a permanent upstream rejection must surface as 400, not 503"
    );
}

#[actix_web::test]
async fn backfill_returns_candles_written_count() {
    let candles = sample_candles("BTC/USDT", &[100, 200, 300]);
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(candles)),
    );
    let state = build_state(sources, Some(Arc::new(FakeCandleRepository::new(vec![]))));
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTC/USDT"))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["data"]["candles_written"], 3);
}

#[actix_web::test]
async fn backfill_persists_candles_then_backtest_run_succeeds() {
    const STRATEGY_ID: &str = "backfill-then-backtest";
    const HOLD_CODE: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    count += 1
    action = "buy" if count == 1 else None
    if action:
        print(json.dumps({
            "signal_id": str(uuid.uuid4()),
            "strategy_id": _STRATEGY_ID,
            "exchange": candle["exchange"],
            "pair": candle["pair"],
            "action": action,
            "confidence": 1.0,
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }), flush=True)
"#;

    let candles = sample_candles("BTC/USDT", &[100, 200, 300]);
    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(candles)),
    );

    let record = PythonStrategyRecord {
        strategy_id: STRATEGY_ID.to_string(),
        name: "Backfill Test Strategy".to_string(),
        code: HOLD_CODE.to_string(),
        params_json: serde_json::json!({}),
        created_at: Utc::now(),
    };
    let python_repo = Arc::new(FakePythonStrategyRepository::with_records(vec![record]));
    let candle_repo: Arc<dyn CandleRepository> = Arc::new(FakeCandleRepository::new(vec![]));

    let state = actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: None,
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: Some(python_repo),
        candle_repository: Some(candle_repo),
        historical_sources: Arc::new(sources),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        asset_repository: None,
        subscription_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    });

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let backfill_resp = srv
        .post("/v1/candles/backfill")
        .send_json(&backfill_request("coinex", "BTC/USDT"))
        .await
        .unwrap();
    assert_eq!(backfill_resp.status(), 200, "backfill must succeed");

    let from = Utc.timestamp_opt(1_699_999_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_100_000, 0).unwrap();
    let mut backtest_resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": STRATEGY_ID,
            "exchange": "coinex",
            "pair": "BTC/USDT",
            "interval": "1h",
            "from": from,
            "to": to,
        }))
        .await
        .unwrap();

    assert_eq!(
        backtest_resp.status(),
        200,
        "backtest must find the candles the backfill just persisted"
    );
    let body: Value = backtest_resp.json().await.unwrap();
    assert_eq!(body["data"]["candle_count"], 3);
}
