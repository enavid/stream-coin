use std::sync::Arc;

use actix_web::{web, HttpResponse};
use chrono::Utc;

use crate::infrastructure::db::strategy_repository::StrategyRecord;
use crate::presentation::dto::strategy::{
    ActiveStrategy, StartStrategyRequest, StopStrategyRequest, StrategyList,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::{AppState, StrategyHandle};
use crate::strategy::builtin::price_delta::PriceDeltaStrategy;
use crate::strategy::builtin::spread_threshold::SpreadThresholdStrategy;
use crate::strategy::port::Strategy;
use crate::strategy::runner::spawn_strategy_runner;

fn build_strategy(record: &StrategyRecord) -> Option<Arc<dyn Strategy>> {
    match record.strategy_type.as_str() {
        "spread_threshold" => {
            let threshold = record.params_json["threshold"].as_u64()?;
            Some(Arc::new(SpreadThresholdStrategy::new(
                &record.strategy_id,
                &record.exchange,
                &record.pair,
                threshold,
            )))
        }
        "price_delta" => {
            let window = record.params_json["window"].as_u64().unwrap_or(5) as usize;
            let threshold = record.params_json["threshold"].as_f64().unwrap_or(0.02);
            Some(Arc::new(PriceDeltaStrategy::new(
                &record.strategy_id,
                &record.exchange,
                &record.pair,
                window,
                threshold,
            )))
        }
        _ => None,
    }
}

pub async fn start_strategy(
    state: web::Data<AppState>,
    body: web::Json<StartStrategyRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    let mut running = state.running_strategies.lock().await;
    if running.contains_key(&req.strategy_id) {
        return ApiError::new(
            &format!("strategy '{}' is already running", req.strategy_id),
            vec![],
        )
        .to_response();
    }

    let record = StrategyRecord {
        strategy_id: req.strategy_id.clone(),
        strategy_type: req.strategy_type.clone(),
        exchange: req.exchange.clone(),
        pair: req.pair.clone(),
        params_json: req.params.clone(),
        started_at: Utc::now(),
    };

    let strategy = match build_strategy(&record) {
        Some(s) => s,
        None => {
            return ApiError::new(
                &format!("unknown strategy type '{}'", req.strategy_type),
                vec![],
            )
            .to_response();
        }
    };

    let abort_handle =
        spawn_strategy_runner(strategy, state.broadcaster.clone(), state.publisher.clone());

    running.insert(
        req.strategy_id.clone(),
        StrategyHandle {
            strategy_type: req.strategy_type.clone(),
            exchange: req.exchange.clone(),
            pair: req.pair.clone(),
            abort_handle,
        },
    );
    drop(running);

    if let Some(ref repo) = state.strategy_repository {
        if let Err(e) = repo.save(&record).await {
            tracing::error!(error = %e, "failed to persist strategy record");
        }
    }

    success_response(
        "Strategy started",
        serde_json::json!({
            "strategy_id": req.strategy_id,
            "strategy_type": req.strategy_type,
        }),
    )
}

pub async fn stop_strategy(
    state: web::Data<AppState>,
    body: web::Json<StopStrategyRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    let mut running = state.running_strategies.lock().await;
    match running.remove(&req.strategy_id) {
        Some(handle) => {
            handle.abort_handle.abort();
            drop(running);

            if let Some(ref repo) = state.strategy_repository {
                if let Err(e) = repo.remove(&req.strategy_id).await {
                    tracing::error!(error = %e, "failed to remove strategy record");
                }
            }

            success_response(
                "Strategy stopped",
                serde_json::json!({"strategy_id": req.strategy_id}),
            )
        }
        None => ApiError::new(
            &format!("strategy '{}' is not running", req.strategy_id),
            vec![],
        )
        .to_response(),
    }
}

pub async fn list_strategies(state: web::Data<AppState>) -> HttpResponse {
    let running = state.running_strategies.lock().await;
    let strategies: Vec<ActiveStrategy> = running
        .iter()
        .map(|(id, h)| ActiveStrategy {
            strategy_id: id.clone(),
            strategy_type: h.strategy_type.clone(),
            exchange: h.exchange.clone(),
            pair: h.pair.clone(),
        })
        .collect();
    success_response("Active strategies", StrategyList { strategies })
}

/// Loads active strategy records from the repository and restarts each one.
/// Called on engine startup to restore state that survived a restart.
pub async fn restore_strategies(state: &web::Data<AppState>) {
    let repo = match &state.strategy_repository {
        Some(r) => r.clone(),
        None => return,
    };

    let records = match repo.list_active().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to load strategies from repository");
            return;
        }
    };

    let mut running = state.running_strategies.lock().await;
    for record in records {
        if running.contains_key(&record.strategy_id) {
            continue;
        }
        let strategy = match build_strategy(&record) {
            Some(s) => s,
            None => {
                tracing::warn!(
                    strategy_type = %record.strategy_type,
                    "unknown strategy type during restore"
                );
                continue;
            }
        };
        let abort_handle =
            spawn_strategy_runner(strategy, state.broadcaster.clone(), state.publisher.clone());
        running.insert(
            record.strategy_id.clone(),
            StrategyHandle {
                strategy_type: record.strategy_type.clone(),
                exchange: record.exchange.clone(),
                pair: record.pair.clone(),
                abort_handle,
            },
        );
        tracing::info!(strategy_id = %record.strategy_id, "restored strategy");
    }
}
