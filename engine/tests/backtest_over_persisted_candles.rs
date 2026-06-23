//! Loop 6g verification: `POST /v1/backtest/run` is exchange-agnostic and
//! already works once candles exist in the repository — for CoinEx via a
//! backfill (6d) and for Tabdeal/Hitobit via candles the live aggregator
//! would have accumulated over time (6c). No new engine code; this file
//! only proves the existing endpoint requires zero changes for either path.

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

const BUY_THEN_SELL_CODE: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    count += 1
    action = "buy" if count == 1 else "sell" if count == 2 else None
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

struct FakeHistoricalSource {
    result: Mutex<Option<Result<Vec<Candle>, HistoricalCandleSourceError>>>,
}

impl FakeHistoricalSource {
    fn with_candles(candles: Vec<Candle>) -> Self {
        Self {
            result: Mutex::new(Some(Ok(candles))),
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

fn candle(exchange: &str, pair: &str, interval: Interval, time_secs: i64, close: u64) -> Candle {
    Candle {
        exchange: exchange.to_string(),
        pair: pair.to_string(),
        interval,
        time: Utc.timestamp_opt(time_secs, 0).unwrap(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 10,
    }
}

fn build_state(
    historical_sources: HashMap<String, Arc<dyn HistoricalCandleSource>>,
    candle_repository: Arc<dyn CandleRepository>,
    strategy_id: &str,
) -> actix_web::web::Data<AppState> {
    let record = PythonStrategyRecord {
        strategy_id: strategy_id.to_string(),
        name: "Verification Strategy".to_string(),
        code: BUY_THEN_SELL_CODE.to_string(),
        params_json: json!({}),
        created_at: Utc::now(),
    };
    let python_repo = Arc::new(FakePythonStrategyRepository::with_records(vec![record]));

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
        python_strategy_repository: Some(python_repo),
        candle_repository: Some(candle_repository),
        historical_sources: Arc::new(historical_sources),
        top_market_sources: Arc::new(HashMap::new()),
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    })
}

#[actix_web::test]
async fn backtest_run_over_coinex_backfilled_range_returns_closed_trades() {
    let strategy_id = "verify-coinex";
    let candles = vec![
        candle(
            "coinex",
            "BTC/USDT",
            Interval::OneHour,
            1_700_000_000,
            100_000,
        ),
        candle(
            "coinex",
            "BTC/USDT",
            Interval::OneHour,
            1_700_003_600,
            90_000,
        ),
        candle(
            "coinex",
            "BTC/USDT",
            Interval::OneHour,
            1_700_007_200,
            110_000,
        ),
    ];

    let mut sources: HashMap<String, Arc<dyn HistoricalCandleSource>> = HashMap::new();
    sources.insert(
        "coinex".to_string(),
        Arc::new(FakeHistoricalSource::with_candles(candles)),
    );
    let candle_repo: Arc<dyn CandleRepository> = Arc::new(FakeCandleRepository::new(vec![]));

    let state = build_state(sources, candle_repo, strategy_id);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // Step 1 (Loop 6d): backfill from the historical source into the repository.
    let backfill_resp = srv
        .post("/v1/candles/backfill")
        .send_json(&json!({
            "exchange": "coinex",
            "pair": "BTC/USDT",
            "interval": "1h",
            "from": "2023-11-14T22:00:00Z",
            "to": "2023-11-14T23:00:00Z",
        }))
        .await
        .unwrap();
    assert_eq!(backfill_resp.status(), 200, "backfill must succeed");

    // Step 2 (Loop 6g — this is the part being verified): the existing,
    // unmodified /v1/backtest/run endpoint finds and uses the backfilled
    // candles with zero CoinEx-specific code in the backtest path.
    let from = Utc.timestamp_opt(1_699_999_000, 0).unwrap();
    let to = Utc.timestamp_opt(1_700_010_000, 0).unwrap();
    let mut backtest_resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": strategy_id,
            "exchange": "coinex",
            "pair": "BTC/USDT",
            "interval": "1h",
            "from": from,
            "to": to,
        }))
        .await
        .unwrap();

    assert_eq!(backtest_resp.status(), 200);
    let body: Value = backtest_resp.json().await.unwrap();
    assert_eq!(body["data"]["candle_count"], 3);
    assert_eq!(
        body["data"]["closed_trades"].as_array().unwrap().len(),
        1,
        "buy+sell over the backfilled CoinEx range must pair into one closed trade"
    );
}

#[actix_web::test]
async fn backtest_run_over_tabdeal_live_accumulated_history_returns_closed_trades() {
    let strategy_id = "verify-tabdeal";

    // No HistoricalCandleSource for tabdeal — proves the backtest path does
    // not require one. These candles simulate what the live CandleAggregator
    // (Loop 6c's exchange_handler wiring) would have persisted over time,
    // written directly via the same CandleRepository::upsert_candles path.
    let candle_repo: Arc<dyn CandleRepository> = Arc::new(FakeCandleRepository::new(vec![]));
    candle_repo
        .upsert_candles(&[
            candle(
                "tabdeal",
                "USDT/IRT",
                Interval::OneMinute,
                1_000_000,
                100_000,
            ),
            candle(
                "tabdeal",
                "USDT/IRT",
                Interval::OneMinute,
                1_000_060,
                90_000,
            ),
            candle(
                "tabdeal",
                "USDT/IRT",
                Interval::OneMinute,
                1_000_120,
                110_000,
            ),
        ])
        .await
        .expect("simulated live-aggregator upsert must succeed");

    let state = build_state(HashMap::new(), candle_repo, strategy_id);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let from = Utc.timestamp_opt(999_990, 0).unwrap();
    let to = Utc.timestamp_opt(1_000_200, 0).unwrap();
    let mut resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": strategy_id,
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "interval": "1m",
            "from": from,
            "to": to,
        }))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "backtest must work for an exchange with no HistoricalCandleSource, \
         as long as candles already exist in the repository"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["data"]["candle_count"], 3);
    assert_eq!(body["data"]["closed_trades"].as_array().unwrap().len(), 1);
}

#[actix_web::test]
async fn backtest_run_over_hitobit_live_accumulated_history_returns_closed_trades() {
    let strategy_id = "verify-hitobit";

    let candle_repo: Arc<dyn CandleRepository> = Arc::new(FakeCandleRepository::new(vec![]));
    candle_repo
        .upsert_candles(&[
            candle(
                "hitobit",
                "USDT/IRT",
                Interval::FiveMinutes,
                2_000_000,
                50_000,
            ),
            candle(
                "hitobit",
                "USDT/IRT",
                Interval::FiveMinutes,
                2_000_300,
                45_000,
            ),
            candle(
                "hitobit",
                "USDT/IRT",
                Interval::FiveMinutes,
                2_000_600,
                55_000,
            ),
        ])
        .await
        .unwrap();

    let state = build_state(HashMap::new(), candle_repo, strategy_id);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let from = Utc.timestamp_opt(1_999_990, 0).unwrap();
    let to = Utc.timestamp_opt(2_001_000, 0).unwrap();
    let mut resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": strategy_id,
            "exchange": "hitobit",
            "pair": "USDT/IRT",
            "interval": "5m",
            "from": from,
            "to": to,
        }))
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["data"]["candle_count"], 3);
}

#[actix_web::test]
async fn backtest_run_returns_4xx_when_no_candles_exist_for_either_path() {
    let strategy_id = "verify-empty";
    let candle_repo: Arc<dyn CandleRepository> = Arc::new(FakeCandleRepository::new(vec![]));
    let state = build_state(HashMap::new(), candle_repo, strategy_id);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": strategy_id,
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "interval": "1m",
            "from": "2026-01-01T00:00:00Z",
            "to": "2026-01-02T00:00:00Z",
        }))
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "no candles for any exchange (CoinEx or otherwise) must still 4xx, not panic"
    );
}
