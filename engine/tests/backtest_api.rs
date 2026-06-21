use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use stream_coin::candle::entity::CandlePayload;
use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::infrastructure::db::candle_repository::FakeCandleRepository;
use stream_coin::infrastructure::db::python_strategy_repository::{
    FakePythonStrategyRepository, PythonStrategyRecord,
};
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

/// Python strategy: buy on candle 1, sell on candle 2, hold thereafter.
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

fn make_candles(closes: &[u64]) -> Vec<CandlePayload> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &close)| CandlePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: Utc.timestamp_opt(1_000_000 + i as i64 * 60, 0).unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 10,
        })
        .collect()
}

fn build_state(
    candles: Vec<CandlePayload>,
    strategy_id: &str,
    code: &str,
) -> actix_web::web::Data<AppState> {
    let record = PythonStrategyRecord {
        strategy_id: strategy_id.to_string(),
        name: "Test Strategy".to_string(),
        code: code.to_string(),
        params_json: serde_json::json!({}),
        created_at: Utc::now(),
    };
    let python_repo = Arc::new(FakePythonStrategyRepository::with_records(vec![record]));
    let candle_repo = Arc::new(FakeCandleRepository::new(candles));

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
        admin_credentials: None,
        order_manager: None,
        python_strategy_repository: Some(python_repo),
        candle_repository: Some(candle_repo),
    })
}

#[actix_web::test]
async fn backtest_result_includes_pnl_and_drawdown() {
    // 3 candles: buy fills at candle[1].close=90K, sell fills at candle[2].close=110K → profit
    let candles = make_candles(&[100_000, 90_000, 110_000]);
    let strategy_id = "backtest-strat-1";
    let state = build_state(candles, strategy_id, BUY_THEN_SELL_CODE);

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

    assert_eq!(resp.status(), 200, "backtest run must return 200");

    let body: Value = resp.json().await.unwrap();
    let data = &body["data"];

    assert!(
        data["total_return_pct"].is_number(),
        "total_return_pct must be a number"
    );
    assert!(
        data["max_drawdown_pct"].is_number(),
        "max_drawdown_pct must be a number"
    );
    assert!(
        data["signal_count"].as_u64().unwrap_or(0) >= 2,
        "at least 2 signals expected (buy + sell)"
    );
    assert!(
        data["candle_count"].as_u64().unwrap() == 3,
        "3 candles must be processed"
    );
    assert!(
        data["trade_log"].as_array().unwrap().len() == 2,
        "2 fills expected (buy and sell)"
    );
    assert!(
        data["total_return_pct"].as_f64().unwrap() > 0.0,
        "buy at 90K, sell at 110K must be profitable"
    );
}

#[actix_web::test]
async fn backtest_produces_same_signals_as_live_for_same_candles() {
    let candles = make_candles(&[100_000, 100_000, 100_000, 100_000, 100_000]);
    let strategy_id = "det-strat";
    let state1 = build_state(candles.clone(), strategy_id, BUY_THEN_SELL_CODE);
    let state2 = build_state(candles, strategy_id, BUY_THEN_SELL_CODE);

    let srv1 =
        actix_test::start(move || App::new().app_data(state1.clone()).configure(init_routes));
    let srv2 =
        actix_test::start(move || App::new().app_data(state2.clone()).configure(init_routes));

    let from = Utc.timestamp_opt(999_990, 0).unwrap();
    let to = Utc.timestamp_opt(1_000_500, 0).unwrap();
    let payload = json!({
        "strategy_id": strategy_id,
        "exchange": "tabdeal",
        "pair": "USDT/IRT",
        "interval": "1m",
        "from": from,
        "to": to,
    });

    let mut resp1 = srv1
        .post("/v1/backtest/run")
        .send_json(&payload)
        .await
        .unwrap();
    let mut resp2 = srv2
        .post("/v1/backtest/run")
        .send_json(&payload)
        .await
        .unwrap();

    let body1: Value = resp1.json().await.unwrap();
    let body2: Value = resp2.json().await.unwrap();

    let count1 = body1["data"]["signal_count"].as_u64().unwrap();
    let count2 = body2["data"]["signal_count"].as_u64().unwrap();
    assert_eq!(
        count1, count2,
        "same strategy + same candles must produce the same number of signals"
    );

    let actions1: Vec<&str> = body1["data"]["signal_log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap())
        .collect();
    let actions2: Vec<&str> = body2["data"]["signal_log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions1, actions2,
        "signal actions must be identical across runs"
    );
}

#[actix_web::test]
async fn backtest_unknown_strategy_returns_4xx() {
    let candle_repo = Arc::new(FakeCandleRepository::new(vec![]));
    let python_repo = Arc::new(FakePythonStrategyRepository::new());

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
        admin_credentials: None,
        order_manager: None,
        python_strategy_repository: Some(python_repo),
        candle_repository: Some(candle_repo),
    });

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": "does-not-exist",
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
        "unknown strategy must return a 4xx status, got {}",
        resp.status()
    );
}

#[actix_web::test]
async fn backtest_invalid_date_range_returns_4xx() {
    let candle_repo = Arc::new(FakeCandleRepository::new(vec![]));
    let python_repo = Arc::new(FakePythonStrategyRepository::new());

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
        admin_credentials: None,
        order_manager: None,
        python_strategy_repository: Some(python_repo),
        candle_repository: Some(candle_repo),
    });

    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    // from > to must be rejected
    let resp = srv
        .post("/v1/backtest/run")
        .send_json(&json!({
            "strategy_id": "any",
            "exchange": "tabdeal",
            "pair": "USDT/IRT",
            "interval": "1m",
            "from": "2026-02-01T00:00:00Z",
            "to": "2026-01-01T00:00:00Z",
        }))
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "'from' after 'to' must return a 4xx status, got {}",
        resp.status()
    );
}
