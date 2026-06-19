use actix_web::{web, Responder};

use crate::presentation::dto::exchange::{
    ExchangeListResponse, ExchangeNameRequest, ExchangeResponse, PairListQuery, PairListResponse,
    PairResponse,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::AppState;

/// `GET /v1/exchanges` — returns all currently enabled exchanges.
pub async fn list_exchanges(state: web::Data<AppState>) -> impl Responder {
    let registry = state.exchange_registry.lock().await;
    let exchanges = registry
        .get_enabled_exchanges()
        .into_iter()
        .map(|e| ExchangeResponse {
            name: e.name.clone(),
            display_name: e.display_name.clone(),
            enabled: e.enabled,
        })
        .collect();

    success_response("Enabled exchanges", ExchangeListResponse { exchanges })
}

/// `GET /v1/exchanges/{name}/pairs` — returns active pairs for an exchange,
/// optionally filtered by `?market_type=spot|futures|swap`.
pub async fn list_exchange_pairs(
    state: web::Data<AppState>,
    path: web::Path<String>,
    query: web::Query<PairListQuery>,
) -> impl Responder {
    let exchange_name = path.into_inner();
    let registry = state.exchange_registry.lock().await;
    let pairs = registry
        .get_active_pairs(&exchange_name, query.market_type.as_ref())
        .into_iter()
        .map(|p| PairResponse {
            base: p.base.clone(),
            quote: p.quote.clone(),
            market_type: p.market_type.clone(),
            active: p.active,
        })
        .collect();

    success_response("Active pairs", PairListResponse { pairs })
}

/// `POST /v1/admin/exchanges/enable` — enables an exchange and inserts its adapter
/// into the live adapter map using the registered factory.
pub async fn enable_exchange(
    state: web::Data<AppState>,
    body: web::Json<ExchangeNameRequest>,
) -> impl Responder {
    let name = body.exchange.as_str();

    let ws_url = {
        let mut registry = state.exchange_registry.lock().await;
        if !registry.enable(name) {
            return ApiError::new("Exchange not found in registry", vec![]).to_response();
        }
        registry.find_ws_url(name).map(String::from)
    };

    let ws_url = match ws_url {
        Some(u) => u,
        None => {
            return ApiError::new("Exchange has no WS URL configured", vec![]).to_response();
        }
    };

    if let Some(factory) = state.adapter_factories.get(name) {
        let adapter = factory(&ws_url);
        state
            .exchange_adapters
            .write()
            .await
            .insert(name.to_string(), adapter);
        tracing::info!(exchange = %name, "exchange enabled and adapter inserted");
    }

    success_response("Exchange enabled", serde_json::json!({"exchange": name}))
}

/// `POST /v1/admin/exchanges/disable` — disables an exchange, removes its adapter,
/// and aborts all running ticker subscriptions for that exchange.
pub async fn disable_exchange(
    state: web::Data<AppState>,
    body: web::Json<ExchangeNameRequest>,
) -> impl Responder {
    let name = body.exchange.as_str();

    {
        let mut registry = state.exchange_registry.lock().await;
        if !registry.disable(name) {
            return ApiError::new("Exchange not found in registry", vec![]).to_response();
        }
    }

    state.exchange_adapters.write().await.remove(name);

    let prefix = format!("{name}:");
    let mut clients = state.clients.lock().await;
    let to_abort: Vec<String> = clients
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    for key in to_abort {
        if let Some(handle) = clients.remove(&key) {
            handle.abort();
        }
    }

    tracing::info!(exchange = %name, "exchange disabled and tickers aborted");
    success_response("Exchange disabled", serde_json::json!({"exchange": name}))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use actix_web::{test, web, App};
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::exchange::registry::{ExchangeRecord, ExchangeRegistry, TradingPairRecord};
    use crate::presentation::shared::app_state::{AdapterFactory, AppState};
    use crate::price::entity::MarketType;

    fn registry_with_exchanges() -> ExchangeRegistry {
        let mut r = ExchangeRegistry::new();
        r.add_exchange(ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled: true,
        });
        r.add_exchange(ExchangeRecord {
            name: "hitobit".to_string(),
            display_name: "Hitobit".to_string(),
            ws_url: "wss://hitobit.example.com".to_string(),
            enabled: false,
        });
        r
    }

    fn state_with_registry(registry: ExchangeRegistry) -> web::Data<AppState> {
        web::Data::new(AppState {
            redis: None,
            exchange_adapters: Arc::new(RwLock::new(HashMap::new())),
            exchange_registry: Arc::new(Mutex::new(registry)),
            adapter_factories: Arc::new(HashMap::<String, AdapterFactory>::new()),
            clients: Arc::new(Mutex::new(HashMap::new())),
            publisher: None,
            broadcaster: AppState::new_broadcaster(),
            jwt_secret: None,
        })
    }

    #[actix_web::test]
    async fn list_exchanges_returns_only_enabled() {
        let app = test::init_service(
            App::new()
                .app_data(state_with_registry(registry_with_exchanges()))
                .route("/exchanges", web::get().to(list_exchanges)),
        )
        .await;

        let req = test::TestRequest::get().uri("/exchanges").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, req).await;
        let exchanges = body["data"]["exchanges"].as_array().unwrap();
        assert_eq!(exchanges.len(), 1);
        assert_eq!(exchanges[0]["name"], "tabdeal");
    }

    #[actix_web::test]
    async fn disable_then_enable_exchange_changes_list() {
        let registry = registry_with_exchanges();
        let state = state_with_registry(registry);

        let app = test::init_service(
            App::new()
                .app_data(state)
                .route("/exchanges", web::get().to(list_exchanges))
                .route("/admin/exchanges/disable", web::post().to(disable_exchange))
                .route("/admin/exchanges/enable", web::post().to(enable_exchange)),
        )
        .await;

        let disable_req = test::TestRequest::post()
            .uri("/admin/exchanges/disable")
            .set_json(serde_json::json!({"exchange": "tabdeal"}))
            .to_request();
        let disable_resp = test::call_service(&app, disable_req).await;
        assert_eq!(disable_resp.status(), 200);

        let list_req = test::TestRequest::get().uri("/exchanges").to_request();
        let body: serde_json::Value = test::call_and_read_body_json(&app, list_req).await;
        assert_eq!(
            body["data"]["exchanges"].as_array().unwrap().len(),
            0,
            "no enabled exchanges after disable"
        );
    }

    #[actix_web::test]
    async fn list_pairs_returns_200() {
        let mut registry = ExchangeRegistry::new();
        registry.add_exchange(ExchangeRecord {
            name: "tabdeal".to_string(),
            display_name: "Tabdeal".to_string(),
            ws_url: "wss://tabdeal.example.com".to_string(),
            enabled: true,
        });
        registry.add_pair(TradingPairRecord {
            exchange_name: "tabdeal".to_string(),
            base: "USDT".to_string(),
            quote: "IRT".to_string(),
            market_type: MarketType::Spot,
            active: true,
        });

        let app = test::init_service(App::new().app_data(state_with_registry(registry)).route(
            "/exchanges/{name}/pairs",
            web::get().to(list_exchange_pairs),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/exchanges/tabdeal/pairs")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
    }
}
