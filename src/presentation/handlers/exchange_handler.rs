use actix_web::{web, HttpResponse};
use sea_orm::{DatabaseConnection, EntityTrait};
use crate::presentation::shared::app_state::AppState;
use crate::presentation::responses::success_response;
use crate::infrastructure::persistence::models::exchange::Entity as Exchange;
use crate::presentation::dto::exchange::{ExchangeNameList};


pub async fn get_exchange_names(
    state: web::Data<AppState>,
) -> HttpResponse {
    let db: &DatabaseConnection = &state.db;

    match Exchange::find().all(db).await {
        Ok(exchanges) => {
            let names = exchanges.into_iter().map(|e| e.name).collect();
            success_response("Exchange names fetched successfully", ExchangeNameList { names })
        }
        Err(_) => HttpResponse::InternalServerError().finish(),
    }
}
