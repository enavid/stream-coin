use actix_web::{web, HttpResponse};
use chrono::Utc;

use crate::infrastructure::db::strategy_repository::{StrategyRecord, StrategyRegistration};
use crate::presentation::dto::strategy::{
    ActiveStrategy, RegisterStrategyRequest, StartStrategyRequest, StopStrategyRequest,
    StrategyList,
};
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::{AppState, StrategyHandle};
use crate::strategy::factory::build_strategy;
use crate::strategy::runner::spawn_strategy_runner;

pub async fn start_strategy(
    state: web::Data<AppState>,
    body: web::Json<StartStrategyRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    // Fast duplicate check before any async work
    {
        let running = state.running_strategies.lock().await;
        if running.contains_key(&req.strategy_id) {
            return ApiError::new(
                &format!("strategy '{}' is already running", req.strategy_id),
                vec![],
            )
            .to_response();
        }
    }

    let strategy = match build_strategy(
        &req.strategy_id,
        &req.strategy_type,
        &req.exchange,
        &req.pair,
        &req.params,
    ) {
        Some(s) => s,
        None => {
            return ApiError::new(
                &format!("unknown strategy type '{}'", req.strategy_type),
                vec![],
            )
            .to_response();
        }
    };

    // Persist first — if this fails, we don't start the runner
    let record = StrategyRecord {
        strategy_id: req.strategy_id.clone(),
        strategy_type: req.strategy_type.clone(),
        exchange: req.exchange.clone(),
        pair: req.pair.clone(),
        params_json: req.params.clone(),
        started_at: Utc::now(),
    };
    if let Some(ref repo) = state.strategy_repository {
        if let Err(e) = repo.save(&record).await {
            tracing::error!(error = %e, strategy_id = %req.strategy_id, "failed to persist strategy record");
            return ApiError::new("failed to persist strategy", vec![]).to_response();
        }
    }

    let abort_handle = spawn_strategy_runner(
        strategy,
        state.broadcaster.clone(),
        state.publisher.clone(),
        state.signal_repository.clone(),
    );

    let mut running = state.running_strategies.lock().await;
    // Double-check after re-acquiring the lock (TOCTOU guard)
    if running.contains_key(&req.strategy_id) {
        abort_handle.abort();
        if let Some(ref repo) = state.strategy_repository {
            let _ = repo.remove(&req.strategy_id).await;
        }
        return ApiError::new(
            &format!("strategy '{}' is already running", req.strategy_id),
            vec![],
        )
        .to_response();
    }
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

    tracing::info!(
        strategy_id = %req.strategy_id,
        strategy_type = %req.strategy_type,
        exchange = %req.exchange,
        pair = %req.pair,
        "strategy started"
    );

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
                    tracing::error!(error = %e, strategy_id = %req.strategy_id, "failed to remove strategy record");
                }
            }

            tracing::info!(strategy_id = %req.strategy_id, "strategy stopped");

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

pub async fn register_strategy(
    state: web::Data<AppState>,
    body: web::Json<RegisterStrategyRequest>,
) -> HttpResponse {
    let req = body.into_inner();

    if req.strategy_type != "builtin" && req.strategy_type != "external" {
        return ApiError::new("strategy_type must be 'builtin' or 'external'", vec![])
            .to_response();
    }

    let reg = StrategyRegistration {
        strategy_id: req.strategy_id.clone(),
        name: req.name.clone(),
        strategy_type: req.strategy_type.clone(),
        registered_at: Utc::now(),
    };

    if let Some(ref repo) = state.strategy_repository {
        if let Err(e) = repo.register(&reg).await {
            tracing::error!(error = %e, strategy_id = %req.strategy_id, "failed to register strategy");
            return ApiError::new("failed to register strategy", vec![]).to_response();
        }
    }

    tracing::info!(
        strategy_id = %req.strategy_id,
        name = %req.name,
        strategy_type = %req.strategy_type,
        "strategy registered"
    );

    success_response(
        "Strategy registered",
        serde_json::json!({
            "strategy_id": req.strategy_id,
            "name": req.name,
            "strategy_type": req.strategy_type,
        }),
    )
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
        let strategy = match build_strategy(
            &record.strategy_id,
            &record.strategy_type,
            &record.exchange,
            &record.pair,
            &record.params_json,
        ) {
            Some(s) => s,
            None => {
                tracing::warn!(
                    strategy_type = %record.strategy_type,
                    "unknown strategy type during restore — skipping"
                );
                continue;
            }
        };
        let abort_handle = spawn_strategy_runner(
            strategy,
            state.broadcaster.clone(),
            state.publisher.clone(),
            state.signal_repository.clone(),
        );
        running.insert(
            record.strategy_id.clone(),
            StrategyHandle {
                strategy_type: record.strategy_type.clone(),
                exchange: record.exchange.clone(),
                pair: record.pair.clone(),
                abort_handle,
            },
        );
        tracing::info!(strategy_id = %record.strategy_id, strategy_type = %record.strategy_type, "strategy restored");
    }
}
