use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;

use crate::infrastructure::db::python_strategy_repository::PythonStrategyRecord;
use crate::infrastructure::db::strategy_repository::{StrategyRecord, StrategyRegistration};
use crate::presentation::dto::strategy::{
    ActiveStrategy, DeployStrategyRequest, DeployedStrategy, RegisterStrategyRequest,
    StartStrategyRequest, StopStrategyRequest, StrategyList,
};
use crate::presentation::middlewares::jwt::require_permission;
use crate::presentation::responses::{success_response, ApiError};
use crate::presentation::shared::app_state::{AppState, StrategyHandle};
use crate::strategy::factory::build_strategy;
use crate::strategy::runner::spawn_strategy_runner;
use crate::strategy::subprocess::{spawn_subprocess_runner, SubprocessConfig};

/// Permission required to manage strategies (start/stop/register/deploy/list) and
/// to run backtests. Deploying and backtesting execute user-supplied Python, so
/// this is a high-trust capability granted to admin and trader roles.
const STRATEGIES_MANAGE: &str = "strategies.manage";

#[utoipa::path(
    post,
    path = "/v1/strategies/start",
    tag = "Strategies",
    request_body = StartStrategyRequest,
    responses(
        (status = 200, description = "Strategy started"),
        (status = 400, description = "Unknown strategy type or already running", body = ApiError),
        (status = 401, description = "Not authenticated or missing permission", body = ApiError)
    )
)]
pub async fn start_strategy(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<StartStrategyRequest>,
) -> HttpResponse {
    if let Err(resp) = require_permission(&req, STRATEGIES_MANAGE) {
        return resp;
    }

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

#[utoipa::path(
    post,
    path = "/v1/strategies/stop",
    tag = "Strategies",
    request_body = StopStrategyRequest,
    responses(
        (status = 200, description = "Strategy stopped"),
        (status = 400, description = "Strategy is not running", body = ApiError),
        (status = 401, description = "Not authenticated or missing permission", body = ApiError)
    )
)]
pub async fn stop_strategy(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<StopStrategyRequest>,
) -> HttpResponse {
    if let Err(resp) = require_permission(&req, STRATEGIES_MANAGE) {
        return resp;
    }

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

#[utoipa::path(
    get,
    path = "/v1/strategies",
    tag = "Strategies",
    responses(
        (status = 200, description = "Active strategies", body = StrategyList),
        (status = 401, description = "Not authenticated or missing permission", body = ApiError)
    )
)]
pub async fn list_strategies(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(resp) = require_permission(&req, STRATEGIES_MANAGE) {
        return resp;
    }

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

#[utoipa::path(
    post,
    path = "/v1/strategies/register",
    tag = "Strategies",
    request_body = RegisterStrategyRequest,
    responses(
        (status = 200, description = "Strategy registered"),
        (status = 400, description = "Invalid strategy_type", body = ApiError),
        (status = 401, description = "Not authenticated or missing permission", body = ApiError)
    )
)]
pub async fn register_strategy(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<RegisterStrategyRequest>,
) -> HttpResponse {
    if let Err(resp) = require_permission(&req, STRATEGIES_MANAGE) {
        return resp;
    }

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

/// Deploys a Python strategy: saves code to the repository and spawns the subprocess.
///
/// The engine prepends a seccomp preamble (Linux) that blocks socket/connect/bind
/// before executing the user's code. The subprocess reads candle JSON lines from
/// stdin and writes signal JSON lines to stdout.
#[utoipa::path(
    post,
    path = "/v1/strategies/deploy",
    tag = "Strategies",
    request_body = DeployStrategyRequest,
    responses(
        (status = 200, description = "Strategy deployed", body = DeployedStrategy),
        (status = 400, description = "Empty name or code", body = ApiError),
        (status = 401, description = "Not authenticated or missing permission", body = ApiError)
    )
)]
pub async fn deploy_strategy(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<DeployStrategyRequest>,
) -> HttpResponse {
    let ctx = match require_permission(&req, STRATEGIES_MANAGE) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let req = body.into_inner();

    if req.name.is_empty() {
        return ApiError::new("name must not be empty", vec![]).to_response();
    }
    if req.code.is_empty() {
        return ApiError::new("code must not be empty", vec![]).to_response();
    }

    let strategy_id = uuid::Uuid::new_v4().to_string();

    {
        let running = state.running_strategies.lock().await;
        // No conflict possible since strategy_id is a fresh UUID — just guard duplicates
        // in case of hash collision (cosmetic, practically impossible)
        if running.contains_key(&strategy_id) {
            return ApiError::new("strategy id collision, retry", vec![]).to_response();
        }
    }

    let record = PythonStrategyRecord {
        strategy_id: strategy_id.clone(),
        name: req.name.clone(),
        code: req.code.clone(),
        params_json: req.params.clone(),
        created_at: Utc::now(),
    };

    if let Some(ref repo) = state.python_strategy_repository {
        if let Err(e) = repo.save(&record).await {
            tracing::error!(
                error = %e,
                strategy_id = %strategy_id,
                "failed to persist python strategy"
            );
            return ApiError::new("failed to persist strategy", vec![]).to_response();
        }
    }

    let abort_handle = spawn_subprocess_runner(
        SubprocessConfig {
            strategy_id: strategy_id.clone(),
            code: req.code.clone(),
        },
        state.broadcaster.clone(),
        state.signal_repository.clone(),
    );

    let mut running = state.running_strategies.lock().await;
    running.insert(
        strategy_id.clone(),
        StrategyHandle {
            strategy_type: "python".to_string(),
            exchange: "*".to_string(),
            pair: "*".to_string(),
            abort_handle,
        },
    );
    drop(running);

    tracing::info!(
        actor_user_id = ctx.user_id,
        strategy_id = %strategy_id,
        name = %req.name,
        "python strategy deployed and subprocess started"
    );

    success_response(
        "Strategy deployed",
        serde_json::json!({
            "strategy_id": strategy_id,
            "name": req.name,
        }),
    )
}

/// Restores deployed Python strategies from the repository on engine startup.
pub async fn restore_python_strategies(state: &web::Data<AppState>) {
    let repo = match &state.python_strategy_repository {
        Some(r) => r.clone(),
        None => return,
    };

    let records = match repo.list_active().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to load python strategies from repository");
            return;
        }
    };

    let mut running = state.running_strategies.lock().await;
    for record in records {
        if running.contains_key(&record.strategy_id) {
            continue;
        }
        let abort_handle = spawn_subprocess_runner(
            SubprocessConfig {
                strategy_id: record.strategy_id.clone(),
                code: record.code.clone(),
            },
            state.broadcaster.clone(),
            state.signal_repository.clone(),
        );
        running.insert(
            record.strategy_id.clone(),
            StrategyHandle {
                strategy_type: "python".to_string(),
                exchange: "*".to_string(),
                pair: "*".to_string(),
                abort_handle,
            },
        );
        tracing::info!(
            strategy_id = %record.strategy_id,
            name = %record.name,
            "python strategy subprocess restored"
        );
    }
}
