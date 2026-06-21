mod auth_router;
mod backtest_router;
mod candle_router;
mod exchange_routers;
mod health_router;
mod order_router;
mod registry_router;
mod strategy_router;
mod user_router;

use actix_web::middleware::from_fn;
use actix_web::web;

use crate::presentation::handlers::ws_handler;
use crate::presentation::middlewares::jwt::jwt_middleware;

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/v1")
            .wrap(from_fn(jwt_middleware))
            .configure(auth_router::auth_router)
            .configure(health_router::health_router)
            .configure(exchange_routers::exchange_router)
            .configure(registry_router::registry_router)
            .configure(strategy_router::strategy_router)
            .configure(order_router::order_router)
            .configure(backtest_router::backtest_router)
            .configure(candle_router::candle_router)
            .configure(user_router::user_router)
            .route("/ws", web::get().to(ws_handler::ws_index)),
    );
}
