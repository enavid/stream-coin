use std::collections::HashMap;
use std::sync::Arc;

use actix_web::App;
use chrono::Utc;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

use stream_coin::candle::entity::CandlePayload;
use stream_coin::exchange::registry::ExchangeRegistry;
use stream_coin::presentation::middlewares::jwt::mint_token;
use stream_coin::presentation::routers::init_routes;
use stream_coin::presentation::shared::app_state::{AdapterFactory, AppState};

const SECRET: &str = "test_jwt_secret";

fn build_state() -> actix_web::web::Data<AppState> {
    actix_web::web::Data::new(AppState {
        redis: None,
        exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
        exchange_registry: Arc::new(Mutex::new(ExchangeRegistry::new())),
        adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
        clients: Arc::new(Mutex::new(HashMap::new())),
        publisher: None,
        broadcaster: AppState::new_broadcaster(),
        jwt_secret: Some(Arc::new(SECRET.to_string())),
        ticker_repository: None,
        running_strategies: Arc::new(Mutex::new(HashMap::new())),
        strategy_repository: None,
        signal_repository: None,
        order_adapters: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        order_manager: None,
        python_strategy_repository: None,
        candle_repository: None,
        candle_history: AppState::new_candle_history(),
        exchange_repository: None,
        user_repository: None,
        credential_repository: None,
        credential_cipher: None,
    })
}

fn sample_candle(close: u64, time_offset_secs: i64) -> CandlePayload {
    CandlePayload {
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        interval: "1m".to_string(),
        time: Utc::now() + chrono::Duration::seconds(time_offset_secs),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1,
    }
}

#[actix_web::test]
async fn get_candles_without_token_returns_401() {
    let state = build_state();
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let resp = srv
        .get("/v1/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "missing token must return 401");
}

#[actix_web::test]
async fn get_candles_returns_seeded_history_ordered_by_time() {
    let state = build_state();
    state.push_candle_history(&sample_candle(100, -120)).await;
    state.push_candle_history(&sample_candle(200, -60)).await;
    state.push_candle_history(&sample_candle(300, 0)).await;

    let token = mint_token("test_user", SECRET, 3600);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut resp = srv
        .get("/v1/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let candles = body["data"].as_array().unwrap();
    assert_eq!(candles.len(), 3);
    assert_eq!(
        candles
            .iter()
            .map(|c| c["close"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![100, 200, 300],
        "must be ordered oldest first"
    );
}

#[actix_web::test]
async fn get_candles_respects_limit_keeping_the_newest() {
    let state = build_state();
    for i in 0..10u64 {
        state
            .push_candle_history(&sample_candle(i, i as i64 - 10))
            .await;
    }

    let token = mint_token("test_user", SECRET, 3600);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut resp = srv
        .get("/v1/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m&limit=3")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let candles = body["data"].as_array().unwrap();
    assert_eq!(
        candles
            .iter()
            .map(|c| c["close"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![7, 8, 9]
    );
}

#[actix_web::test]
async fn get_candles_for_unknown_pair_returns_empty_array() {
    let state = build_state();
    let token = mint_token("test_user", SECRET, 3600);
    let srv = actix_test::start(move || App::new().app_data(state.clone()).configure(init_routes));

    let mut resp = srv
        .get("/v1/candles?exchange=tabdeal&pair=USDT/IRT&interval=1m")
        .insert_header(("Authorization", format!("Bearer {token}")))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["data"].as_array().unwrap().is_empty());
}
